// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: OAuth 2.1 / PKCE (RFC 7636), Protected Resource Metadata (RFC 9728),
//   and Authorization Server Metadata (RFC 8414) support for remote MCP servers.
// - Security: Enforces HTTPS on all non-loopback OAuth endpoints, forbids embedded URL
//   credentials, enforces S256 PKCE, and keeps bearer tokens out of Core descriptors.

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::{form_urlencoded, Host, Url};
use uuid::Uuid;

const MAX_OAUTH_RESPONSE_BYTES: u64 = 1_024 * 1_024;
const PKCE_VERIFIER_MIN_LEN: usize = 43;
const PKCE_VERIFIER_MAX_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthError {
    pub code: &'static str,
    pub message: String,
}

impl OAuthError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for OAuthError {}

/// RFC 7636 PKCE verifier and S256 challenge pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
    pub code_challenge_method: &'static str,
}

impl PkcePair {
    /// Generates a cryptographically random RFC 7636 PKCE verifier (64 URL-safe chars,
    /// 384 bits of entropy) and its S256 challenge.
    pub fn generate() -> Self {
        let mut entropy = [0_u8; 48];
        fill_cryptographic_bytes(&mut entropy);
        let code_verifier = base64url_encode_no_pad(&entropy);
        let code_challenge = compute_s256_challenge(&code_verifier);
        Self {
            code_verifier,
            code_challenge,
            code_challenge_method: "S256",
        }
    }

    /// Constructs a PKCE S256 pair from an existing RFC 7636-compliant verifier.
    pub fn from_verifier(verifier: impl Into<String>) -> Result<Self, OAuthError> {
        let code_verifier = verifier.into();
        validate_pkce_verifier(&code_verifier)?;
        let code_challenge = compute_s256_challenge(&code_verifier);
        Ok(Self {
            code_verifier,
            code_challenge,
            code_challenge_method: "S256",
        })
    }
}

/// Computes `BASE64URL-ENCODE(SHA256(ASCII(verifier)))` without padding per RFC 7636.
pub fn compute_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64url_encode_no_pad(&digest)
}

fn validate_pkce_verifier(verifier: &str) -> Result<(), OAuthError> {
    let len = verifier.len();
    if !(PKCE_VERIFIER_MIN_LEN..=PKCE_VERIFIER_MAX_LEN).contains(&len) {
        return Err(OAuthError::new(
            "oauth_pkce_invalid",
            format!(
                "PKCE code_verifier length must be between {PKCE_VERIFIER_MIN_LEN} and {PKCE_VERIFIER_MAX_LEN} characters"
            ),
        ));
    }
    if !verifier
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(OAuthError::new(
            "oauth_pkce_invalid",
            "PKCE code_verifier contains characters outside RFC 7636 unreserved set",
        ));
    }
    Ok(())
}

fn fill_cryptographic_bytes(out: &mut [u8]) {
    #[cfg(unix)]
    {
        if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
            let _ = file.read_exact(out);
        }
    }
    // Mix with OS CSPRNG bytes from Uuid::new_v4() (backed by getrandom) so all
    // platforms receive full cryptographic randomness even if /dev/urandom is absent.
    for chunk in out.chunks_mut(16) {
        let random_bytes = Uuid::new_v4().into_bytes();
        for (dst, src) in chunk.iter_mut().zip(random_bytes.iter()) {
            *dst ^= *src;
        }
    }
}

pub(crate) fn base64url_encode_no_pad(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((input.len() * 4 + 2) / 3);
    let mut chunks = input.chunks_exact(3);
    for chunk in &mut chunks {
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];
        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        output.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        output.push(TABLE[(b2 & 0x3f) as usize] as char);
    }
    match chunks.remainder() {
        [b0] => {
            output.push(TABLE[(b0 >> 2) as usize] as char);
            output.push(TABLE[((b0 & 0x03) << 4) as usize] as char);
        }
        [b0, b1] => {
            output.push(TABLE[(b0 >> 2) as usize] as char);
            output.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            output.push(TABLE[((b1 & 0x0f) << 2) as usize] as char);
        }
        _ => {}
    }
    output
}

/// RFC 9728 OAuth 2.0 Protected Resource Metadata (`/.well-known/oauth-protected-resource`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    #[serde(default)]
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub bearer_methods_supported: Vec<String>,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub resource_documentation: Option<String>,
}

/// RFC 8414 OAuth 2.0 Authorization Server Metadata (`/.well-known/oauth-authorization-server`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

/// Stored OAuth 2.1 bearer token set for an MCP server connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    pub access_token: String,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub expires_at_epoch_secs: Option<u64>,
}

impl OAuthTokenSet {
    /// Returns true if the token has an expiration timestamp and is expired (or within
    /// `skew_secs` of expiring) relative to `now_epoch_secs`.
    pub fn is_expired_with_skew(&self, now_epoch_secs: u64, skew_secs: u64) -> bool {
        self.expires_at_epoch_secs
            .is_some_and(|expires_at| now_epoch_secs.saturating_add(skew_secs) >= expires_at)
    }

    /// Returns true if the token is expired (or within `skew_secs` of expiring) right now.
    pub fn is_expired_now(&self, skew_secs: u64) -> bool {
        self.is_expired_with_skew(current_epoch_secs(), skew_secs)
    }

    /// Constructs the validated `Authorization: Bearer <access_token>` HTTP header value.
    pub fn bearer_header_value(&self) -> Result<HeaderValue, OAuthError> {
        let token = self.access_token.trim();
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(OAuthError::new(
                "oauth_token_invalid",
                "OAuth access token is empty or contains invalid header characters",
            ));
        }
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
            OAuthError::new(
                "oauth_token_invalid",
                "OAuth access token cannot be encoded in an Authorization header",
            )
        })
    }

    /// Parses an OAuth 2.1 token endpoint JSON response into an `OAuthTokenSet`.
    pub fn from_token_response(payload: &Value, now_epoch_secs: u64) -> Result<Self, OAuthError> {
        let access_token = payload
            .get("access_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                OAuthError::new(
                    "oauth_token_invalid",
                    "OAuth token response is missing access_token",
                )
            })?
            .to_string();
        let token_type = payload
            .get("token_type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("Bearer")
            .to_string();
        let refresh_token = payload
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let scope = payload
            .get("scope")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let expires_at_epoch_secs = payload
            .get("expires_at_epoch_secs")
            .and_then(Value::as_u64)
            .or_else(|| {
                payload
                    .get("expires_in")
                    .and_then(Value::as_u64)
                    .map(|ttl| now_epoch_secs.saturating_add(ttl))
            });
        let token_set = Self {
            access_token,
            token_type,
            refresh_token,
            scope,
            expires_at_epoch_secs,
        };
        token_set.bearer_header_value()?;
        Ok(token_set)
    }
}

/// Structured representation of an HTTP `WWW-Authenticate: Bearer ...` challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WwwAuthenticateChallenge {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_metadata: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// Parses RFC 9728 `WWW-Authenticate: Bearer resource_metadata="..."` and returns the
/// `resource_metadata` URL if present in a Bearer challenge.
pub fn parse_www_authenticate_resource_metadata(header_value: &str) -> Option<String> {
    parse_www_authenticate_challenge(header_value)?
        .resource_metadata
        .filter(|value| !value.trim().is_empty())
}

/// Parses the `Bearer` challenge parameters from a `WWW-Authenticate` header value.
pub fn parse_www_authenticate_challenge(header_value: &str) -> Option<WwwAuthenticateChallenge> {
    let mut challenges: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let bytes = header_value.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        skip_ws_and_commas(bytes, &mut cursor);
        if cursor >= bytes.len() {
            break;
        }
        let token = read_token(bytes, &mut cursor);
        if token.is_empty() {
            cursor += 1;
            continue;
        }
        skip_ws(bytes, &mut cursor);
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            // Parameter belonging to the current challenge.
            cursor += 1;
            skip_ws(bytes, &mut cursor);
            let value = read_auth_param_value(bytes, &mut cursor);
            if let Some((_, params)) = challenges.last_mut() {
                params.push((token.to_ascii_lowercase(), value));
            }
        } else {
            // Start of a new auth-scheme challenge.
            challenges.push((token, Vec::new()));
            // Check if followed by a bare token68 (not key=value) before next comma.
            let lookahead = cursor;
            let next_tok = read_token(bytes, &mut cursor);
            let after_tok = cursor;
            skip_ws(bytes, &mut cursor);
            if !next_tok.is_empty() && cursor < bytes.len() && bytes[cursor] == b'=' {
                // Actually an auth-param (`key=...`); rewind so main loop parses it.
                cursor = lookahead;
            } else if !next_tok.is_empty() {
                // Could be a token68 or another challenge; advance past token68 if followed by ',' or EOF.
                let mut rest = after_tok;
                while rest < bytes.len() && bytes[rest] == b'=' {
                    rest += 1;
                }
                skip_ws(bytes, &mut rest);
                if rest >= bytes.len() || bytes[rest] == b',' {
                    cursor = rest;
                } else {
                    cursor = lookahead;
                }
            }
        }
    }

    let (_, params) = challenges
        .into_iter()
        .find(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))?;

    let mut challenge = WwwAuthenticateChallenge {
        scheme: "Bearer".to_string(),
        resource_metadata: None,
        realm: None,
        scope: None,
        error: None,
        error_description: None,
    };
    for (key, value) in params {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        match key.as_str() {
            "resource_metadata" => challenge.resource_metadata = Some(trimmed.to_string()),
            "realm" => challenge.realm = Some(trimmed.to_string()),
            "scope" => challenge.scope = Some(trimmed.to_string()),
            "error" => challenge.error = Some(trimmed.to_string()),
            "error_description" => challenge.error_description = Some(trimmed.to_string()),
            _ => {}
        }
    }
    Some(challenge)
}

/// Extracts the best `Bearer` challenge across all `WWW-Authenticate` headers in a response.
pub fn parse_www_authenticate_headers(headers: &HeaderMap) -> Option<WwwAuthenticateChallenge> {
    let mut fallback = None;
    for value in headers.get_all(WWW_AUTHENTICATE) {
        let Ok(text) = value.to_str() else {
            continue;
        };
        if let Some(challenge) = parse_www_authenticate_challenge(text) {
            if challenge.resource_metadata.is_some() {
                return Some(challenge);
            }
            if fallback.is_none() {
                fallback = Some(challenge);
            }
        }
    }
    fallback
}

fn skip_ws(bytes: &[u8], cursor: &mut usize) {
    while *cursor < bytes.len() && matches!(bytes[*cursor], b' ' | b'\t') {
        *cursor += 1;
    }
}

fn skip_ws_and_commas(bytes: &[u8], cursor: &mut usize) {
    while *cursor < bytes.len() && matches!(bytes[*cursor], b' ' | b'\t' | b',') {
        *cursor += 1;
    }
}

fn read_token(bytes: &[u8], cursor: &mut usize) -> String {
    let start = *cursor;
    while *cursor < bytes.len()
        && !matches!(bytes[*cursor], b' ' | b'\t' | b'=' | b',' | b'"')
    {
        *cursor += 1;
    }
    String::from_utf8_lossy(&bytes[start..*cursor]).into_owned()
}

fn read_auth_param_value(bytes: &[u8], cursor: &mut usize) -> String {
    if *cursor < bytes.len() && bytes[*cursor] == b'"' {
        *cursor += 1;
        let mut out = Vec::new();
        while *cursor < bytes.len() {
            let b = bytes[*cursor];
            *cursor += 1;
            if b == b'\\' && *cursor < bytes.len() {
                out.push(bytes[*cursor]);
                *cursor += 1;
            } else if b == b'"' {
                break;
            } else {
                out.push(b);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    } else {
        let start = *cursor;
        while *cursor < bytes.len() && !matches!(bytes[*cursor], b',' | b' ' | b'\t') {
            *cursor += 1;
        }
        String::from_utf8_lossy(&bytes[start..*cursor]).into_owned()
    }
}

/// Validates an OAuth endpoint URL: enforces HTTPS for all remote hosts, permits HTTP
/// only for loopback addresses (`localhost`, `127.0.0.1`, `[::1]`) during local testing,
/// and forbids embedded username/password credentials.
pub fn validate_oauth_endpoint_url(raw: &str) -> Result<Url, OAuthError> {
    let parsed = Url::parse(raw.trim()).map_err(|_| {
        OAuthError::new(
            "oauth_url_invalid",
            "OAuth endpoint must be a valid absolute URL",
        )
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OAuthError::new(
            "oauth_url_credentials_forbidden",
            "OAuth endpoint URL must not contain embedded credentials",
        ));
    }
    let is_loopback = is_loopback_host(&parsed);
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if is_loopback => Ok(parsed),
        "http" => Err(OAuthError::new(
            "oauth_https_required",
            "OAuth endpoints must use HTTPS (HTTP is allowed only for localhost)",
        )),
        _ => Err(OAuthError::new(
            "oauth_url_invalid",
            "OAuth endpoint must use https (or http for localhost)",
        )),
    }
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn validate_redirect_uri(raw: &str) -> Result<Url, OAuthError> {
    let parsed = Url::parse(raw.trim()).map_err(|_| {
        OAuthError::new(
            "oauth_redirect_uri_invalid",
            "OAuth redirect_uri must be a valid absolute URI",
        )
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OAuthError::new(
            "oauth_redirect_uri_invalid",
            "OAuth redirect_uri must not contain embedded credentials",
        ));
    }
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if is_loopback_host(&parsed) => Ok(parsed),
        "http" => Err(OAuthError::new(
            "oauth_https_required",
            "HTTP redirect_uri is only permitted for loopback addresses",
        )),
        "javascript" | "data" | "file" | "vbscript" | "blob" => Err(OAuthError::new(
            "oauth_redirect_uri_invalid",
            "OAuth redirect_uri scheme is not allowed",
        )),
        _ => Ok(parsed),
    }
}

/// Derives the RFC 9728 well-known protected resource metadata URL for a resource URI.
pub fn protected_resource_metadata_url(resource_url: &str) -> Result<Url, OAuthError> {
    well_known_metadata_url(resource_url, "/.well-known/oauth-protected-resource")
}

/// Derives the RFC 8414 well-known authorization server metadata URL for an issuer URI.
pub fn authorization_server_metadata_url(issuer_url: &str) -> Result<Url, OAuthError> {
    well_known_metadata_url(issuer_url, "/.well-known/oauth-authorization-server")
}

fn well_known_metadata_url(base_url: &str, well_known_prefix: &str) -> Result<Url, OAuthError> {
    let parsed = validate_oauth_endpoint_url(base_url)?;
    let origin = parsed.origin().ascii_serialization();
    let trimmed_path = parsed.path().trim_end_matches('/');
    let full = if trimmed_path.is_empty() || trimmed_path == "/" {
        format!("{origin}{well_known_prefix}")
    } else {
        format!("{origin}{well_known_prefix}{trimmed_path}")
    };
    validate_oauth_endpoint_url(&full)
}

/// Builds an OAuth 2.1 authorization URL with PKCE S256 parameters and optional RFC 8707
/// `resource` indicator.
pub fn build_authorization_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    pkce: &PkcePair,
    state: &str,
    scopes: &[&str],
    resource: Option<&str>,
) -> Result<String, OAuthError> {
    let mut url = validate_oauth_endpoint_url(authorization_endpoint)?;
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err(OAuthError::new(
            "oauth_parameter_invalid",
            "OAuth client_id must not be empty",
        ));
    }
    let redirect = validate_redirect_uri(redirect_uri)?;
    let state = state.trim();
    if state.is_empty() {
        return Err(OAuthError::new(
            "oauth_parameter_invalid",
            "OAuth state parameter must not be empty",
        ));
    }
    validate_pkce_verifier(&pkce.code_verifier)?;
    if pkce.code_challenge.trim().is_empty() || pkce.code_challenge_method != "S256" {
        return Err(OAuthError::new(
            "oauth_pkce_invalid",
            "OAuth 2.1 requires PKCE with S256 code_challenge_method",
        ));
    }
    let validated_resource = match resource.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(validate_oauth_endpoint_url(value)?),
        None => None,
    };
    let joined_scopes: Vec<&str> = scopes
        .iter()
        .map(|scope| scope.trim())
        .filter(|scope| !scope.is_empty())
        .collect();

    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("redirect_uri", redirect.as_str());
        pairs.append_pair("code_challenge", pkce.code_challenge.trim());
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("state", state);
        if !joined_scopes.is_empty() {
            pairs.append_pair("scope", &joined_scopes.join(" "));
        }
        if let Some(res) = validated_resource {
            pairs.append_pair("resource", res.as_str());
        }
    }

    Ok(url.into())
}

/// Builds a blocking HTTP client suitable for OAuth discovery and token exchanges
/// (no ambient proxy, bounded timeouts, same-origin redirects disabled to avoid leaking
/// tokens/codes across origins).
pub fn build_oauth_http_client(timeout_ms: u64) -> Result<Client, OAuthError> {
    let clamped = timeout_ms.clamp(1_000, 60_000);
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(clamped.min(15_000)))
        .timeout(Duration::from_millis(clamped))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| OAuthError::new("oauth_client_failed", error.to_string()))
}

/// Fetches RFC 9728 Protected Resource Metadata from `metadata_url`.
pub fn fetch_protected_resource_metadata(
    client: &Client,
    metadata_url: &str,
) -> Result<ProtectedResourceMetadata, OAuthError> {
    let url = validate_oauth_endpoint_url(metadata_url)?;
    let body = fetch_json_document(client, &url)?;
    let metadata: ProtectedResourceMetadata = serde_json::from_str(&body).map_err(|error| {
        OAuthError::new(
            "oauth_metadata_invalid",
            format!("Invalid protected resource metadata JSON: {error}"),
        )
    })?;
    if metadata.resource.trim().is_empty() {
        return Err(OAuthError::new(
            "oauth_metadata_invalid",
            "Protected resource metadata is missing resource identifier",
        ));
    }
    Ok(metadata)
}

/// Fetches RFC 8414 Authorization Server Metadata from an issuer URL or explicit
/// `/.well-known/oauth-authorization-server` URL.
pub fn fetch_authorization_server_metadata(
    client: &Client,
    issuer_or_metadata_url: &str,
) -> Result<AuthorizationServerMetadata, OAuthError> {
    let parsed = validate_oauth_endpoint_url(issuer_or_metadata_url)?;
    let url = if parsed.path().contains("/.well-known/") {
        parsed
    } else {
        authorization_server_metadata_url(parsed.as_str())?
    };
    let body = fetch_json_document(client, &url)?;
    let metadata: AuthorizationServerMetadata = serde_json::from_str(&body).map_err(|error| {
        OAuthError::new(
            "oauth_metadata_invalid",
            format!("Invalid authorization server metadata JSON: {error}"),
        )
    })?;
    validate_oauth_endpoint_url(&metadata.authorization_endpoint)?;
    validate_oauth_endpoint_url(&metadata.token_endpoint)?;
    if !metadata.code_challenge_methods_supported.is_empty()
        && !metadata
            .code_challenge_methods_supported
            .iter()
            .any(|method| method == "S256")
    {
        return Err(OAuthError::new(
            "oauth_pkce_unsupported",
            "Authorization server does not advertise PKCE S256 support",
        ));
    }
    Ok(metadata)
}

/// Exchanges an OAuth 2.1 authorization code + PKCE `code_verifier` for an `OAuthTokenSet`.
#[allow(clippy::too_many_arguments)]
pub fn exchange_code_for_token(
    client: &Client,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    resource: Option<&str>,
) -> Result<OAuthTokenSet, OAuthError> {
    let url = validate_oauth_endpoint_url(token_endpoint)?;
    let client_id = client_id.trim();
    let code = code.trim();
    if client_id.is_empty() || code.is_empty() {
        return Err(OAuthError::new(
            "oauth_parameter_invalid",
            "OAuth token exchange requires non-empty client_id and code",
        ));
    }
    let redirect = validate_redirect_uri(redirect_uri)?;
    validate_pkce_verifier(code_verifier)?;

    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "authorization_code");
    form.append_pair("code", code);
    form.append_pair("redirect_uri", redirect.as_str());
    form.append_pair("client_id", client_id);
    form.append_pair("code_verifier", code_verifier);
    if let Some(secret) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
        form.append_pair("client_secret", secret);
    }
    if let Some(res) = resource.map(str::trim).filter(|s| !s.is_empty()) {
        let validated = validate_oauth_endpoint_url(res)?;
        form.append_pair("resource", validated.as_str());
    }
    let encoded_body = form.finish();
    post_token_request(client, &url, encoded_body, None)
}

/// Refreshes an OAuth 2.1 access token using a `refresh_token`. Preserves the existing
/// `refresh_token` if the authorization server does not issue a rotated one.
pub fn refresh_access_token(
    client: &Client,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    scopes: Option<&[&str]>,
    resource: Option<&str>,
) -> Result<OAuthTokenSet, OAuthError> {
    let url = validate_oauth_endpoint_url(token_endpoint)?;
    let client_id = client_id.trim();
    let refresh_token = refresh_token.trim();
    if client_id.is_empty() || refresh_token.is_empty() {
        return Err(OAuthError::new(
            "oauth_parameter_invalid",
            "OAuth token refresh requires non-empty client_id and refresh_token",
        ));
    }

    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "refresh_token");
    form.append_pair("refresh_token", refresh_token);
    form.append_pair("client_id", client_id);
    if let Some(secret) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
        form.append_pair("client_secret", secret);
    }
    if let Some(scopes) = scopes {
        let joined: Vec<&str> = scopes
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !joined.is_empty() {
            form.append_pair("scope", &joined.join(" "));
        }
    }
    if let Some(res) = resource.map(str::trim).filter(|s| !s.is_empty()) {
        let validated = validate_oauth_endpoint_url(res)?;
        form.append_pair("resource", validated.as_str());
    }
    let encoded_body = form.finish();
    post_token_request(client, &url, encoded_body, Some(refresh_token))
}

fn fetch_json_document(client: &Client, url: &Url) -> Result<String, OAuthError> {
    let response = client
        .get(url.as_str())
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, concat!("Nola/", env!("CARGO_PKG_VERSION")))
        .send()
        .map_err(|error| OAuthError::new("oauth_request_failed", error.to_string()))?;
    let status = response.status();
    let body = read_bounded_body(response)?;
    if !status.is_success() {
        return Err(OAuthError::new(
            "oauth_http_failed",
            format!(
                "OAuth metadata endpoint returned HTTP {}: {}",
                status.as_u16(),
                truncate(&body, 300)
            ),
        ));
    }
    Ok(body)
}

fn post_token_request(
    client: &Client,
    url: &Url,
    form_body: String,
    fallback_refresh_token: Option<&str>,
) -> Result<OAuthTokenSet, OAuthError> {
    let response = client
        .post(url.as_str())
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(USER_AGENT, concat!("Nola/", env!("CARGO_PKG_VERSION")))
        .body(form_body)
        .send()
        .map_err(|error| OAuthError::new("oauth_request_failed", error.to_string()))?;
    let status = response.status();
    let body = read_bounded_body(response)?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|json| {
                let err = json.get("error").and_then(Value::as_str)?;
                let desc = json.get("error_description").and_then(Value::as_str);
                Some(match desc {
                    Some(d) if !d.is_empty() => format!("{err}: {d}"),
                    _ => err.to_string(),
                })
            })
            .unwrap_or_else(|| truncate(&body, 300));
        return Err(OAuthError::new(
            "oauth_token_exchange_failed",
            format!("OAuth token endpoint returned HTTP {}: {detail}", status.as_u16()),
        ));
    }
    let payload: Value = serde_json::from_str(&body).map_err(|error| {
        OAuthError::new(
            "oauth_token_invalid",
            format!("Invalid OAuth token JSON response: {error}"),
        )
    })?;
    let mut token_set = OAuthTokenSet::from_token_response(&payload, current_epoch_secs())?;
    if token_set.refresh_token.is_none() {
        token_set.refresh_token = fallback_refresh_token.map(str::to_string);
    }
    Ok(token_set)
}

fn read_bounded_body(response: reqwest::blocking::Response) -> Result<String, OAuthError> {
    let mut body = String::new();
    response
        .take(MAX_OAUTH_RESPONSE_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| OAuthError::new("oauth_response_read_failed", error.to_string()))?;
    if body.len() as u64 > MAX_OAUTH_RESPONSE_BYTES {
        return Err(OAuthError::new(
            "oauth_response_too_large",
            "OAuth response exceeded the 1 MB limit",
        ));
    }
    Ok(body)
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write as _;
    use std::net::TcpListener;

    #[test]
    fn pkce_s256_matches_rfc_7636_appendix_b_test_vector() {
        // RFC 7636 Appendix B exact test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        let pair = PkcePair::from_verifier(verifier).expect("valid RFC 7636 verifier");
        assert_eq!(pair.code_verifier, verifier);
        assert_eq!(pair.code_challenge, expected_challenge);
        assert_eq!(pair.code_challenge_method, "S256");

        let generated_a = PkcePair::generate();
        let generated_b = PkcePair::generate();
        assert_ne!(generated_a.code_verifier, generated_b.code_verifier);
        assert!((43..=128).contains(&generated_a.code_verifier.len()));
        assert_eq!(
            generated_a.code_challenge,
            compute_s256_challenge(&generated_a.code_verifier)
        );
        assert!(!generated_a.code_challenge.contains('='));
        assert!(!generated_a.code_challenge.contains('+'));
        assert!(!generated_a.code_challenge.contains('/'));

        assert_eq!(
            PkcePair::from_verifier("too-short").unwrap_err().code,
            "oauth_pkce_invalid"
        );
    }

    #[test]
    fn parses_www_authenticate_resource_metadata_and_bearer_params() {
        let single = r#"Bearer resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource""#;
        assert_eq!(
            parse_www_authenticate_resource_metadata(single).as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );

        let multi = r#"Basic realm="fallback", bearer realm="mcp", RESOURCE_METADATA="https://api.example.com/.well-known/oauth-protected-resource/v1", scope="mcp:read mcp:write", error="insufficient_scope", error_description="Need \"write\" scope""#;
        let parsed = parse_www_authenticate_challenge(multi).expect("Bearer challenge");
        assert_eq!(parsed.scheme, "Bearer");
        assert_eq!(
            parsed.resource_metadata.as_deref(),
            Some("https://api.example.com/.well-known/oauth-protected-resource/v1")
        );
        assert_eq!(parsed.realm.as_deref(), Some("mcp"));
        assert_eq!(parsed.scope.as_deref(), Some("mcp:read mcp:write"));
        assert_eq!(parsed.error.as_deref(), Some("insufficient_scope"));
        assert_eq!(
            parsed.error_description.as_deref(),
            Some(r#"Need "write" scope"#)
        );

        let unquoted = "Bearer resource_metadata=https://mcp.example.com/.well-known/oauth-protected-resource, scope=read";
        assert_eq!(
            parse_www_authenticate_resource_metadata(unquoted).as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );

        let basic_only = r#"Basic realm="admin", resource_metadata="https://ignored.example""#;
        assert!(parse_www_authenticate_resource_metadata(basic_only).is_none());
    }

    #[test]
    fn oauth_token_expiry_skew_and_bearer_validation() {
        let token = OAuthTokenSet {
            access_token: "tok_12345".to_string(),
            token_type: "Bearer".to_string(),
            refresh_token: Some("ref_67890".to_string()),
            scope: Some("mcp:tools".to_string()),
            expires_at_epoch_secs: Some(1_700_000_100),
        };
        assert!(!token.is_expired_with_skew(1_700_000_000, 60));
        assert!(token.is_expired_with_skew(1_700_000_040, 60));
        assert!(token.is_expired_with_skew(1_700_000_100, 0));
        assert_eq!(
            token.bearer_header_value().unwrap().to_str().unwrap(),
            "Bearer tok_12345"
        );

        let non_expiring = OAuthTokenSet {
            access_token: "static_token".to_string(),
            token_type: "Bearer".to_string(),
            refresh_token: None,
            scope: None,
            expires_at_epoch_secs: None,
        };
        assert!(!non_expiring.is_expired_with_skew(u64::MAX, 60));

        let bad_token = OAuthTokenSet {
            access_token: "bad\r\nInjected: yes".to_string(),
            token_type: "Bearer".to_string(),
            refresh_token: None,
            scope: None,
            expires_at_epoch_secs: None,
        };
        assert_eq!(
            bad_token.bearer_header_value().unwrap_err().code,
            "oauth_token_invalid"
        );
    }

    #[test]
    fn builds_oauth21_authorization_url_and_enforces_https() {
        let pkce = PkcePair::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").unwrap();
        let auth_url = build_authorization_url(
            "https://auth.example.com/authorize?prompt=consent",
            "nola-desktop",
            "http://127.0.0.1:43123/oauth/callback",
            &pkce,
            "state-xyz",
            &["mcp:tools", "mcp:resources"],
            Some("https://mcp.example.com/v1"),
        )
        .expect("authorization URL should build");

        let parsed = Url::parse(&auth_url).unwrap();
        let params: std::collections::BTreeMap<String, String> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(params.get("prompt").map(String::as_str), Some("consent"));
        assert_eq!(params.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("nola-desktop")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:43123/oauth/callback")
        );
        assert_eq!(
            params.get("code_challenge").map(String::as_str),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(params.get("state").map(String::as_str), Some("state-xyz"));
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("mcp:tools mcp:resources")
        );
        assert_eq!(
            params.get("resource").map(String::as_str),
            Some("https://mcp.example.com/v1")
        );

        let err = build_authorization_url(
            "http://auth.example.com/authorize",
            "nola-desktop",
            "http://127.0.0.1:43123/oauth/callback",
            &pkce,
            "state-xyz",
            &[],
            None,
        )
        .expect_err("non-localhost HTTP must be rejected");
        assert_eq!(err.code, "oauth_https_required");

        assert_eq!(
            protected_resource_metadata_url("https://mcp.example.com/v1/sse")
                .unwrap()
                .as_str(),
            "https://mcp.example.com/.well-known/oauth-protected-resource/v1/sse"
        );
        assert_eq!(
            authorization_server_metadata_url("https://auth.example.com/")
                .unwrap()
                .as_str(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn metadata_discovery_and_token_exchange_and_refresh_over_localhost() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind OAuth test server");
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let base_for_server = base.clone();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.expect("accept OAuth request");
                let mut req = Vec::new();
                let mut buf = [0_u8; 4096];
                let head_end = loop {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                    if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&req[..head_end]).into_owned();
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        let (name, val) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| val.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or_default();
                while req.len() < head_end + content_length {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let body = String::from_utf8_lossy(
                    &req[head_end..req.len().min(head_end + content_length)],
                );
                let first_line = head.lines().next().unwrap_or("");
                let response_body = if first_line.starts_with("GET /.well-known/oauth-protected-resource ") {
                    json!({
                        "resource": format!("{base_for_server}/mcp"),
                        "authorization_servers": [base_for_server],
                        "scopes_supported": ["mcp:tools"]
                    })
                    .to_string()
                } else if first_line.starts_with("GET /.well-known/oauth-authorization-server ") {
                    json!({
                        "issuer": base_for_server,
                        "authorization_endpoint": format!("{base_for_server}/authorize"),
                        "token_endpoint": format!("{base_for_server}/token"),
                        "code_challenge_methods_supported": ["S256"]
                    })
                    .to_string()
                } else if first_line.starts_with("POST /token ") && body.contains("grant_type=authorization_code") {
                    assert!(body.contains("code=auth-code-1"));
                    assert!(body.contains("code_verifier=dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
                    json!({
                        "access_token": "access-token-1",
                        "token_type": "Bearer",
                        "refresh_token": "refresh-token-1",
                        "expires_in": 3600,
                        "scope": "mcp:tools"
                    })
                    .to_string()
                } else if first_line.starts_with("POST /token ") && body.contains("grant_type=refresh_token") {
                    assert!(body.contains("refresh_token=refresh-token-1"));
                    json!({
                        "access_token": "access-token-2",
                        "token_type": "Bearer",
                        "expires_in": 7200
                    })
                    .to_string()
                } else {
                    panic!("unexpected OAuth request: {first_line} body={body}");
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )
                .unwrap();
            }
        });

        let client = build_oauth_http_client(5_000).unwrap();
        let prm_url = protected_resource_metadata_url(&base).unwrap();
        let prm = fetch_protected_resource_metadata(&client, prm_url.as_str()).unwrap();
        assert_eq!(prm.authorization_servers, vec![base.clone()]);

        let asm = fetch_authorization_server_metadata(&client, &prm.authorization_servers[0]).unwrap();
        assert_eq!(asm.token_endpoint, format!("{base}/token"));

        let exchanged = exchange_code_for_token(
            &client,
            &asm.token_endpoint,
            "nola-client",
            None,
            "auth-code-1",
            "http://127.0.0.1:43123/cb",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            Some(&prm.resource),
        )
        .unwrap();
        assert_eq!(exchanged.access_token, "access-token-1");
        assert_eq!(exchanged.refresh_token.as_deref(), Some("refresh-token-1"));
        assert!(exchanged.expires_at_epoch_secs.is_some());

        let refreshed = refresh_access_token(
            &client,
            &asm.token_endpoint,
            "nola-client",
            None,
            exchanged.refresh_token.as_deref().unwrap(),
            Some(&["mcp:tools"]),
            Some(&prm.resource),
        )
        .unwrap();
        assert_eq!(refreshed.access_token, "access-token-2");
        assert_eq!(
            refreshed.refresh_token.as_deref(),
            Some("refresh-token-1"),
            "refresh_token should be preserved when server does not rotate it"
        );
        server.join().unwrap();
    }
}
