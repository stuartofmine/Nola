// SPDX-License-Identifier: Apache-2.0
//! MCP OAuth 2.1 PKCE, RFC 9728/8414 metadata discovery, RFC 7591 dynamic client
//! registration, interactive loopback authorization-code flow, and single-flight
//! token refresh wired directly to `nola-store::Vault`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nola_store::Vault;
use nola_types::now_ms;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use crate::transport::is_loopback_url;
use crate::McpError;

/// Default maximum time to wait for the user's browser to hit the loopback OAuth callback.
pub const DEFAULT_OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_OAUTH_RESPONSE_BYTES: u64 = 256 * 1024;
const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_endpoint: String,
    pub client_id: String,
    pub expires_at_ms: Option<i64>,
}

/// Discovered OAuth 2.1 authorization server metadata (RFC 9728 + RFC 8414).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub resource: Option<String>,
}

/// RFC 7591 Dynamic Client Registration response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicClientRegistration {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_id_issued_at: Option<i64>,
    #[serde(default)]
    pub client_secret_expires_at: Option<i64>,
}

fn is_disallowed_oauth_ip(ip: IpAddr, allow_loopback: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return !allow_loopback;
            }
            let o = v4.octets();
            v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64.0.0/10
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // Benchmark 198.18.0.0/15
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return !allow_loopback;
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_disallowed_oauth_ip(IpAddr::V4(v4), allow_loopback);
            }
            let segs = v6.segments();
            v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
                || (segs[0] & 0xffc0) == 0xfe80 // Link-local fe80::/10
        }
    }
}

/// Validate an OAuth endpoint URL against SSRF targets and return DNS-pinned socket addresses.
/// Remote (non-loopback) targets require HTTPS and block loopback, private, link-local,
/// CGNAT, and cloud metadata addresses. Loopback targets are only allowed when `allow_loopback`
/// is true (e.g. local dev MCP servers or loopback tests).
fn validate_oauth_url(
    raw_url: &str,
    allow_loopback: bool,
) -> Result<(Url, String, Vec<SocketAddr>), McpError> {
    let parsed = Url::parse(raw_url)
        .map_err(|e| McpError::OAuth(format!("invalid oauth url '{raw_url}': {e}")))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(McpError::OAuth(
            "oauth url must not embed credentials".into(),
        ));
    }
    let target_is_loopback = is_loopback_url(raw_url);
    match parsed.scheme() {
        "https" => {}
        "http" if allow_loopback && target_is_loopback => {}
        "http" => {
            return Err(McpError::OAuth(format!(
                "oauth endpoint '{raw_url}' must use https"
            )));
        }
        other => {
            return Err(McpError::OAuth(format!(
                "unsupported oauth url scheme '{other}'"
            )));
        }
    }
    let host_str = parsed
        .host_str()
        .ok_or_else(|| McpError::OAuth("oauth url missing host".into()))?;
    let host_lc = host_str.trim_end_matches('.').to_ascii_lowercase();
    if matches!(
        host_lc.as_str(),
        "metadata.google.internal" | "metadata.goog" | "169.254.169.254"
    ) {
        return Err(McpError::OAuth(format!(
            "blocked SSRF target: cloud metadata host '{host_str}'"
        )));
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let (host, addrs): (String, Vec<SocketAddr>) = match parsed.host() {
        Some(Host::Ipv4(v4)) => (v4.to_string(), vec![SocketAddr::new(IpAddr::V4(v4), port)]),
        Some(Host::Ipv6(v6)) => (v6.to_string(), vec![SocketAddr::new(IpAddr::V6(v6), port)]),
        Some(Host::Domain(dom)) => {
            let resolved: Vec<SocketAddr> = (dom, port)
                .to_socket_addrs()
                .map_err(|e| McpError::OAuth(format!("dns resolution failed for '{dom}': {e}")))?
                .collect();
            (dom.to_string(), resolved)
        }
        None => return Err(McpError::OAuth("oauth url missing host".into())),
    };
    if addrs.is_empty() {
        return Err(McpError::OAuth(format!(
            "dns returned no addresses for '{host}'"
        )));
    }
    for addr in &addrs {
        if is_disallowed_oauth_ip(addr.ip(), allow_loopback) {
            return Err(McpError::OAuth(format!(
                "blocked SSRF target: '{host}' resolved to disallowed address {}",
                addr.ip()
            )));
        }
    }
    Ok((parsed, host, addrs))
}

fn build_pinned_oauth_client(
    raw_url: &str,
    allow_loopback: bool,
) -> Result<(Client, Url), McpError> {
    let (parsed, host, addrs) = validate_oauth_url(raw_url, allow_loopback)?;
    let mut builder = Client::builder()
        .timeout(OAUTH_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &addrs);
    if is_loopback_url(raw_url) {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| McpError::OAuth(e.to_string()))?;
    Ok((client, parsed))
}

fn read_bounded_json(resp: reqwest::blocking::Response) -> Result<Value, McpError> {
    let mut body = Vec::new();
    resp.take(MAX_OAUTH_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| McpError::OAuth(format!("failed reading oauth response body: {e}")))?;
    if body.len() as u64 > MAX_OAUTH_RESPONSE_BYTES {
        return Err(McpError::OAuth("oauth response exceeded 256 KiB limit".into()));
    }
    serde_json::from_slice(&body)
        .map_err(|e| McpError::OAuth(format!("invalid oauth json response: {e}")))
}

fn origin_of(url: &Url) -> Result<String, McpError> {
    let host = url
        .host_str()
        .ok_or_else(|| McpError::OAuth("url missing host".into()))?;
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

fn well_known_candidates(base: &Url, well_known_name: &str) -> Result<Vec<String>, McpError> {
    let origin = origin_of(base)?;
    let trimmed_path = base.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !trimmed_path.is_empty() && trimmed_path != "/" {
        out.push(format!("{origin}/.well-known/{well_known_name}{trimmed_path}"));
    }
    out.push(format!("{origin}/.well-known/{well_known_name}"));
    Ok(out)
}

fn fetch_well_known_json(
    candidates: &[String],
    allow_loopback: bool,
) -> Result<Option<Value>, McpError> {
    let mut last_err: Option<McpError> = None;
    for candidate in candidates {
        let (client, parsed) = build_pinned_oauth_client(candidate, allow_loopback)?;
        let resp = match client
            .get(parsed)
            .header("accept", "application/json")
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(McpError::OAuth(format!(
                    "oauth metadata GET '{candidate}' failed: {e}"
                )));
                continue;
            }
        };
        if resp.status().as_u16() == 404 {
            continue;
        }
        if !resp.status().is_success() {
            last_err = Some(McpError::OAuth(format!(
                "oauth metadata GET '{candidate}' returned HTTP {}",
                resp.status()
            )));
            continue;
        }
        return Ok(Some(read_bounded_json(resp)?));
    }
    if let Some(err) = last_err {
        return Err(err);
    }
    Ok(None)
}

fn string_list(val: &Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
        .map(ToString::to_string)
        .collect()
}

fn non_empty_str(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Discover OAuth 2.1 metadata for an MCP server by probing
/// `/.well-known/oauth-protected-resource` (RFC 9728) and
/// `/.well-known/oauth-authorization-server` (RFC 8414) with SSRF validation.
pub fn discover_oauth_metadata(server_url: &str) -> Result<OAuthMetadata, McpError> {
    let allow_loopback = is_loopback_url(server_url);
    let (parsed_server, _, _) = validate_oauth_url(server_url, allow_loopback)?;

    // Step 1: RFC 9728 Protected Resource Metadata (optional if server only exposes RFC 8414)
    let prm_candidates = well_known_candidates(&parsed_server, "oauth-protected-resource")?;
    let prm = fetch_well_known_json(&prm_candidates, allow_loopback)?;

    let mut prm_scopes = Vec::new();
    let mut prm_resource = None;
    let auth_server_url = if let Some(prm_val) = prm.as_ref() {
        prm_scopes = string_list(prm_val, "scopes_supported");
        prm_resource = non_empty_str(prm_val, "resource");
        if let Some(first_as) = string_list(prm_val, "authorization_servers").into_iter().next() {
            let (as_parsed, _, _) = validate_oauth_url(&first_as, allow_loopback)?;
            as_parsed
        } else {
            Url::parse(&origin_of(&parsed_server)?)
                .map_err(|e| McpError::OAuth(e.to_string()))?
        }
    } else {
        Url::parse(&origin_of(&parsed_server)?).map_err(|e| McpError::OAuth(e.to_string()))?
    };

    // Step 2: RFC 8414 Authorization Server Metadata
    let as_candidates = well_known_candidates(&auth_server_url, "oauth-authorization-server")?;
    let as_meta = fetch_well_known_json(&as_candidates, allow_loopback)?.ok_or_else(|| {
        McpError::OAuth(format!(
            "authorization server metadata not found at /.well-known/oauth-authorization-server for '{auth_server_url}'"
        ))
    })?;

    let authorization_endpoint = non_empty_str(&as_meta, "authorization_endpoint")
        .ok_or_else(|| McpError::OAuth("oauth metadata missing authorization_endpoint".into()))?;
    let token_endpoint = non_empty_str(&as_meta, "token_endpoint")
        .ok_or_else(|| McpError::OAuth("oauth metadata missing token_endpoint".into()))?;
    let registration_endpoint = non_empty_str(&as_meta, "registration_endpoint");

    // SSRF-validate all discovered endpoints against the original server's loopback policy
    validate_oauth_url(&authorization_endpoint, allow_loopback)?;
    validate_oauth_url(&token_endpoint, allow_loopback)?;
    if let Some(reg) = registration_endpoint.as_deref() {
        validate_oauth_url(reg, allow_loopback)?;
    }

    let issuer = non_empty_str(&as_meta, "issuer")
        .unwrap_or_else(|| auth_server_url.as_str().trim_end_matches('/').to_string());
    let mut scopes_supported = string_list(&as_meta, "scopes_supported");
    if scopes_supported.is_empty() {
        scopes_supported = prm_scopes;
    }
    let code_challenge_methods_supported = string_list(&as_meta, "code_challenge_methods_supported");

    Ok(OAuthMetadata {
        issuer,
        authorization_endpoint,
        token_endpoint,
        registration_endpoint,
        scopes_supported,
        code_challenge_methods_supported,
        resource: prm_resource,
    })
}

/// Perform RFC 7591 Dynamic Client Registration (DCR) at `registration_endpoint`.
pub fn register_dynamic_client(
    registration_endpoint: &str,
    redirect_uri: &str,
    client_name: &str,
) -> Result<DynamicClientRegistration, McpError> {
    let allow_loopback = is_loopback_url(registration_endpoint);
    let (client, parsed) = build_pinned_oauth_client(registration_endpoint, allow_loopback)?;
    let body = json!({
        "client_name": client_name,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let resp = client
        .post(parsed)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .json(&body)
        .send()
        .map_err(|e| McpError::OAuth(format!("dynamic client registration failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().unwrap_or_default();
        return Err(McpError::OAuth(format!(
            "dynamic client registration returned HTTP {status}: {}",
            err_body.chars().take(240).collect::<String>()
        )));
    }
    let json_val = read_bounded_json(resp)?;
    let client_id = non_empty_str(&json_val, "client_id")
        .ok_or_else(|| McpError::OAuth("DCR response missing non-empty client_id".into()))?;
    Ok(DynamicClientRegistration {
        client_id,
        client_secret: non_empty_str(&json_val, "client_secret"),
        client_id_issued_at: json_val.get("client_id_issued_at").and_then(Value::as_i64),
        client_secret_expires_at: json_val
            .get("client_secret_expires_at")
            .and_then(Value::as_i64),
    })
}

/// Exchange an OAuth 2.1 authorization code + PKCE `code_verifier` for tokens.
pub fn exchange_authorization_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenSet, McpError> {
    let request_started_ms = now_ms();
    let allow_loopback = is_loopback_url(token_endpoint);
    let (client, parsed) = build_pinned_oauth_client(token_endpoint, allow_loopback)?;

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", code_verifier),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret));
    }

    let resp = client
        .post(parsed)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .map_err(|e| McpError::OAuth(format!("authorization code exchange failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().unwrap_or_default();
        return Err(McpError::OAuth(format!(
            "token endpoint returned HTTP {status}: {}",
            err_body.chars().take(240).collect::<String>()
        )));
    }
    let body = read_bounded_json(resp)?;
    let access_token = non_empty_str(&body, "access_token")
        .ok_or_else(|| McpError::OAuth("token response missing non-empty access_token".into()))?;
    let refresh_token = non_empty_str(&body, "refresh_token");
    let expires_at_ms = body
        .get("expires_in")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .filter(|&secs| secs > 0)
        .map(|secs| request_started_ms.saturating_add(secs.saturating_mul(1000)));

    Ok(OAuthTokenSet {
        access_token,
        refresh_token,
        token_endpoint: token_endpoint.to_string(),
        client_id: client_id.to_string(),
        expires_at_ms,
    })
}

fn generate_random_state() -> Result<String, McpError> {
    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(|e| McpError::OAuth(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let ha = Sha256::digest(a);
    let hb = Sha256::digest(b);
    let mut diff = 0u8;
    for (x, y) in ha.iter().zip(hb.iter()) {
        diff |= x ^ y;
    }
    diff == 0 && a.len() == b.len()
}

fn send_http_html(stream: &mut std::net::TcpStream, status_line: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// Run an interactive OAuth 2.1 Authorization Code + PKCE flow against `server_url`:
/// 1. Discovers RFC 9728 / RFC 8414 metadata
/// 2. Binds a loopback listener on `127.0.0.1:0`
/// 3. Registers a dynamic client (RFC 7591) if needed
/// 4. Invokes `on_auth_url(url)` so the caller/UI can open the browser
/// 5. Waits for `/callback?code=...&state=...` with strict constant-time state verification
/// 6. Exchanges the code for tokens and persists them into `Vault` under `mcp_oauth:<server_id>`
pub fn run_interactive_oauth_flow<F>(
    server_id: &str,
    server_url: &str,
    vault: Arc<Vault>,
    on_auth_url: F,
) -> Result<OAuthTokenSet, McpError>
where
    F: FnOnce(&str),
{
    run_interactive_oauth_flow_with_options(
        server_id,
        server_url,
        vault,
        None,
        &[],
        DEFAULT_OAUTH_CALLBACK_TIMEOUT,
        on_auth_url,
    )
}

/// Interactive OAuth 2.1 flow with explicit optional `client_id`, `scopes`, and callback `timeout`.
pub fn run_interactive_oauth_flow_with_options<F>(
    server_id: &str,
    server_url: &str,
    vault: Arc<Vault>,
    client_id: Option<&str>,
    scopes: &[String],
    timeout: Duration,
    on_auth_url: F,
) -> Result<OAuthTokenSet, McpError>
where
    F: FnOnce(&str),
{
    let metadata = discover_oauth_metadata(server_url)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let (resolved_client_id, client_secret) = match client_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => (cid.to_string(), None),
        None => {
            let reg_ep = metadata.registration_endpoint.as_deref().ok_or_else(|| {
                McpError::OAuth(
                    "OAuth server has no registration_endpoint and no client_id was supplied"
                        .into(),
                )
            })?;
            let reg = register_dynamic_client(reg_ep, &redirect_uri, "Nola")?;
            (reg.client_id, reg.client_secret)
        }
    };

    let pkce = McpOAuthManager::generate_pkce()?;
    let expected_state = generate_random_state()?;

    let mut auth_url = Url::parse(&metadata.authorization_endpoint)
        .map_err(|e| McpError::OAuth(format!("invalid authorization_endpoint: {e}")))?;
    {
        let mut qp = auth_url.query_pairs_mut();
        qp.append_pair("response_type", "code");
        qp.append_pair("client_id", &resolved_client_id);
        qp.append_pair("redirect_uri", &redirect_uri);
        qp.append_pair("code_challenge", &pkce.challenge);
        qp.append_pair("code_challenge_method", &pkce.method);
        qp.append_pair("state", &expected_state);
        let effective_scopes: Vec<&str> = if !scopes.is_empty() {
            scopes.iter().map(String::as_str).collect()
        } else {
            metadata
                .scopes_supported
                .iter()
                .map(String::as_str)
                .collect()
        };
        if !effective_scopes.is_empty() {
            qp.append_pair("scope", &effective_scopes.join(" "));
        }
        if let Some(res) = metadata.resource.as_deref() {
            qp.append_pair("resource", res);
        }
    }

    on_auth_url(auth_url.as_str());

    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let code = loop {
        if Instant::now() >= deadline {
            return Err(McpError::OAuth(
                "timed out waiting for OAuth loopback callback".into(),
            ));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let n = match stream.read(&mut buf) {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };
                let req_text = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_text.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");
                if !method.eq_ignore_ascii_case("GET") {
                    send_http_html(&mut stream, "405 Method Not Allowed", "Method Not Allowed");
                    continue;
                }
                let callback_url = match Url::parse(&format!("http://127.0.0.1{target}")) {
                    Ok(u) => u,
                    Err(_) => {
                        send_http_html(&mut stream, "400 Bad Request", "Invalid callback URL");
                        continue;
                    }
                };
                if callback_url.path() != "/callback" {
                    send_http_html(&mut stream, "404 Not Found", "Not Found");
                    continue;
                }
                let params: BTreeMap<String, String> = callback_url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();

                if let Some(oauth_err) = params.get("error") {
                    let desc = params
                        .get("error_description")
                        .map(String::as_str)
                        .unwrap_or("authorization denied");
                    send_http_html(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body><h1>Authorization failed</h1></body></html>",
                    );
                    return Err(McpError::OAuth(format!(
                        "OAuth provider returned error '{oauth_err}': {desc}"
                    )));
                }

                let returned_state = params.get("state").map(String::as_str).unwrap_or("");
                if !constant_time_eq(returned_state.as_bytes(), expected_state.as_bytes()) {
                    send_http_html(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body><h1>Invalid OAuth state</h1></body></html>",
                    );
                    return Err(McpError::OAuth(
                        "OAuth callback state verification failed".into(),
                    ));
                }

                let Some(auth_code) = params
                    .get("code")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                else {
                    send_http_html(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body><h1>Missing authorization code</h1></body></html>",
                    );
                    return Err(McpError::OAuth(
                        "OAuth callback missing authorization code".into(),
                    ));
                };

                send_http_html(
                    &mut stream,
                    "200 OK",
                    "<html><body><h1>Nola MCP authorization complete</h1><p>You may close this tab.</p></body></html>",
                );
                break auth_code;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(McpError::Io(e)),
        }
    };

    let tokens = exchange_authorization_code(
        &metadata.token_endpoint,
        &resolved_client_id,
        client_secret.as_deref(),
        &code,
        &pkce.verifier,
        &redirect_uri,
    )?;
    let oauth_mgr = McpOAuthManager::new(vault);
    oauth_mgr.store_tokens(server_id, &tokens)?;
    Ok(tokens)
}

/// Manages PKCE generation, encrypted vault persistence, and single-flight token refresh.
pub struct McpOAuthManager {
    vault: Arc<Vault>,
    refresh_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
}

impl McpOAuthManager {
    pub fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            refresh_locks: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn generate_pkce() -> Result<PkceChallenge, McpError> {
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| McpError::OAuth(e.to_string()))?;
        let verifier = URL_SAFE_NO_PAD.encode(raw);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(PkceChallenge {
            verifier,
            challenge,
            method: "S256".into(),
        })
    }

    fn vault_key(server_id: &str) -> String {
        format!("mcp_oauth:{server_id}")
    }

    fn server_lock(&self, server_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.refresh_locks.lock().unwrap();
        Arc::clone(
            locks
                .entry(server_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn store_tokens_unlocked(&self, server_id: &str, tokens: &OAuthTokenSet) -> Result<(), McpError> {
        let raw = serde_json::to_string(tokens)?;
        self.vault
            .set_secret(&Self::vault_key(server_id), &raw)
            .map_err(|e| McpError::OAuth(e.to_string()))
    }

    pub fn store_tokens(&self, server_id: &str, tokens: &OAuthTokenSet) -> Result<(), McpError> {
        let server_lock = self.server_lock(server_id);
        let _guard = server_lock.lock().unwrap();
        self.store_tokens_unlocked(server_id, tokens)
    }

    pub fn load_tokens(&self, server_id: &str) -> Result<Option<OAuthTokenSet>, McpError> {
        let raw = self
            .vault
            .get_secret(&Self::vault_key(server_id))
            .map_err(|e| McpError::OAuth(e.to_string()))?;
        match raw {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    }

    /// Return a valid access token, automatically performing a single-flight refresh if expiring.
    pub fn valid_access_token(&self, server_id: &str) -> Result<Option<String>, McpError> {
        let Some(tokens) = self.load_tokens(server_id)? else {
            return Ok(None);
        };
        let expiring = tokens
            .expires_at_ms
            .map(|exp| now_ms().saturating_add(60_000) >= exp)
            .unwrap_or(false);
        if !expiring {
            return Ok(Some(tokens.access_token));
        }

        // Single-flight per-server lock so concurrent requests never double-spend a refresh token.
        let server_lock = self.server_lock(server_id);
        let _guard = server_lock.lock().unwrap();

        // Re-check after acquiring lock in case another caller just refreshed!
        let latest = match self.load_tokens(server_id)? {
            Some(t) => t,
            None => return Ok(None),
        };
        let still_expiring = latest
            .expires_at_ms
            .map(|exp| now_ms().saturating_add(60_000) >= exp)
            .unwrap_or(false);
        if !still_expiring {
            return Ok(Some(latest.access_token));
        }

        let Some(refresh_token) = latest.refresh_token.as_deref().filter(|s| !s.is_empty()) else {
            let already_expired = latest
                .expires_at_ms
                .map(|exp| now_ms() >= exp)
                .unwrap_or(false);
            if already_expired {
                return Err(McpError::OAuth(
                    "access token expired and no refresh_token available".into(),
                ));
            }
            return Ok(Some(latest.access_token));
        };
        let refreshed = self.refresh_access_token(&latest, refresh_token)?;
        self.store_tokens_unlocked(server_id, &refreshed)?;
        Ok(Some(refreshed.access_token))
    }

    fn refresh_access_token(
        &self,
        existing: &OAuthTokenSet,
        refresh_token: &str,
    ) -> Result<OAuthTokenSet, McpError> {
        let request_started_ms = now_ms();
        let allow_loopback = is_loopback_url(&existing.token_endpoint);
        let (client, parsed) = build_pinned_oauth_client(&existing.token_endpoint, allow_loopback)?;
        let resp = client
            .post(parsed)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", existing.client_id.as_str()),
            ])
            .send()
            .map_err(|e| McpError::OAuth(format!("token refresh request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().unwrap_or_default();
            return Err(McpError::OAuth(format!(
                "token refresh returned HTTP {status}: {}",
                err_body.chars().take(240).collect::<String>()
            )));
        }
        let body = read_bounded_json(resp)?;
        let access_token = non_empty_str(&body, "access_token").ok_or_else(|| {
            McpError::OAuth("refresh response missing non-empty access_token".into())
        })?;
        let new_refresh = non_empty_str(&body, "refresh_token")
            .or_else(|| existing.refresh_token.clone());
        let expires_at_ms = body
            .get("expires_in")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .filter(|&secs| secs > 0)
            .map(|secs| request_started_ms.saturating_add(secs.saturating_mul(1000)));
        Ok(OAuthTokenSet {
            access_token,
            refresh_token: new_refresh,
            token_endpoint: existing.token_endpoint.clone(),
            client_id: existing.client_id.clone(),
            expires_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_raw_http(stream: &mut std::net::TcpStream) -> (String, String, String) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break buf.len();
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let first_line = header_str.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let content_len = header_str
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buf.len() < header_end + content_len {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let body = if content_len > 0 {
            String::from_utf8_lossy(&buf[header_end..header_end + content_len]).to_string()
        } else {
            String::new()
        };
        (method, path, body)
    }

    fn write_json_http(stream: &mut std::net::TcpStream, status: &str, body: &Value) {
        let s = body.to_string();
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{s}",
            s.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    }

    #[test]
    fn pkce_and_vault_token_storage_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::open(dir.path().join("vault.json")).unwrap());
        let mgr = McpOAuthManager::new(vault);
        let pkce = McpOAuthManager::generate_pkce().unwrap();
        assert_eq!(pkce.method, "S256");
        assert!(!pkce.verifier.is_empty());
        assert_ne!(pkce.verifier, pkce.challenge);

        let tokens = OAuthTokenSet {
            access_token: "tok_live_123".into(),
            refresh_token: Some("ref_456".into()),
            token_endpoint: "https://auth.example.com/token".into(),
            client_id: "nola-cli".into(),
            expires_at_ms: Some(now_ms() + 3_600_000),
        };
        mgr.store_tokens("github", &tokens).unwrap();
        assert_eq!(
            mgr.valid_access_token("github").unwrap().as_deref(),
            Some("tok_live_123")
        );
    }

    #[test]
    fn oauth_ssrf_blocks_metadata_private_ips_and_remote_http() {
        assert!(discover_oauth_metadata("http://169.254.169.254/mcp").is_err());
        assert!(discover_oauth_metadata("https://169.254.169.254/mcp").is_err());
        assert!(discover_oauth_metadata("https://10.0.0.1/mcp").is_err());
        assert!(discover_oauth_metadata("https://192.168.1.5/mcp").is_err());
        assert!(discover_oauth_metadata("https://metadata.google.internal/mcp").is_err());
        assert!(discover_oauth_metadata("http://example.com/mcp").is_err());
        assert!(validate_oauth_url("http://127.0.0.1:8080/token", false).is_err());
        assert!(validate_oauth_url("http://127.0.0.1:8080/token", true).is_ok());
    }

    #[test]
    fn discover_oauth_metadata_rfc9728_and_rfc8414_plus_ssrf_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let base_srv = base.clone();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let (_method, path, _body) = read_raw_http(&mut stream);
                match path.as_str() {
                    "/.well-known/oauth-protected-resource/mcp" => {
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "resource": format!("{base_srv}/mcp"),
                                "authorization_servers": [format!("{base_srv}/auth")],
                                "scopes_supported": ["mcp:read", "mcp:write"]
                            }),
                        );
                    }
                    "/.well-known/oauth-authorization-server/auth" => {
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "issuer": format!("{base_srv}/auth"),
                                "authorization_endpoint": format!("{base_srv}/auth/authorize"),
                                "token_endpoint": format!("{base_srv}/auth/token"),
                                "registration_endpoint": format!("{base_srv}/auth/register"),
                                "code_challenge_methods_supported": ["S256"]
                            }),
                        );
                    }
                    other => panic!("unexpected path: {other}"),
                }
            }
        });

        let meta = discover_oauth_metadata(&format!("{base}/mcp")).unwrap();
        assert_eq!(meta.issuer, format!("{base}/auth"));
        assert_eq!(meta.authorization_endpoint, format!("{base}/auth/authorize"));
        assert_eq!(meta.token_endpoint, format!("{base}/auth/token"));
        assert_eq!(
            meta.registration_endpoint.as_deref(),
            Some(format!("{base}/auth/register").as_str())
        );
        assert_eq!(meta.scopes_supported, vec!["mcp:read", "mcp:write"]);
        assert_eq!(meta.resource.as_deref(), Some(format!("{base}/mcp").as_str()));
        server.join().unwrap();

        // Malicious protected resource metadata pointing to AWS metadata IP must be blocked
        let bad_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bad_addr = bad_listener.local_addr().unwrap();
        let bad_server = thread::spawn(move || {
            if let Ok((mut stream, _)) = bad_listener.accept() {
                let _ = read_raw_http(&mut stream);
                write_json_http(
                    &mut stream,
                    "200 OK",
                    &json!({
                        "authorization_servers": ["http://169.254.169.254/latest/meta-data"]
                    }),
                );
            }
        });
        let err = discover_oauth_metadata(&format!("http://{bad_addr}/")).unwrap_err();
        assert!(err.to_string().contains("SSRF") || err.to_string().contains("https"));
        bad_server.join().unwrap();
    }

    #[test]
    fn register_dynamic_client_and_exchange_authorization_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let (method, path, body) = read_raw_http(&mut stream);
                assert_eq!(method, "POST");
                match path.as_str() {
                    "/register" => {
                        let req: Value = serde_json::from_str(&body).unwrap();
                        assert_eq!(req["client_name"], "Nola");
                        assert_eq!(req["redirect_uris"][0], "http://127.0.0.1:9999/callback");
                        write_json_http(
                            &mut stream,
                            "201 Created",
                            &json!({
                                "client_id": "dcr_client_42",
                                "client_secret": "dcr_secret_99",
                                "client_id_issued_at": 1700000000
                            }),
                        );
                    }
                    "/token" => {
                        let parsed: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        assert_eq!(parsed.get("grant_type").map(String::as_str), Some("authorization_code"));
                        assert_eq!(parsed.get("code").map(String::as_str), Some("auth_code_xyz"));
                        assert_eq!(parsed.get("code_verifier").map(String::as_str), Some("verifier_abc"));
                        assert_eq!(parsed.get("client_id").map(String::as_str), Some("dcr_client_42"));
                        assert_eq!(parsed.get("client_secret").map(String::as_str), Some("dcr_secret_99"));
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "access_token": "acc_live_777",
                                "refresh_token": "ref_live_888",
                                "expires_in": 3600
                            }),
                        );
                    }
                    other => panic!("unexpected path: {other}"),
                }
            }
        });

        let reg = register_dynamic_client(
            &format!("{base}/register"),
            "http://127.0.0.1:9999/callback",
            "Nola",
        )
        .unwrap();
        assert_eq!(reg.client_id, "dcr_client_42");
        assert_eq!(reg.client_secret.as_deref(), Some("dcr_secret_99"));

        let tok = exchange_authorization_code(
            &format!("{base}/token"),
            &reg.client_id,
            reg.client_secret.as_deref(),
            "auth_code_xyz",
            "verifier_abc",
            "http://127.0.0.1:9999/callback",
        )
        .unwrap();
        assert_eq!(tok.access_token, "acc_live_777");
        assert_eq!(tok.refresh_token.as_deref(), Some("ref_live_888"));
        assert!(tok.expires_at_ms.is_some());
        server.join().unwrap();
    }

    #[test]
    fn interactive_oauth_flow_persists_tokens_and_enforces_state_and_timeout() {
        // 1. Happy path: discovery -> DCR -> callback -> token exchange -> Vault persistence
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let base_srv = base.clone();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let (_method, path, body) = read_raw_http(&mut stream);
                match path.as_str() {
                    "/.well-known/oauth-protected-resource" => {
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "authorization_servers": [base_srv]
                            }),
                        );
                    }
                    "/.well-known/oauth-authorization-server" => {
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "issuer": base_srv,
                                "authorization_endpoint": format!("{base_srv}/authorize"),
                                "token_endpoint": format!("{base_srv}/token"),
                                "registration_endpoint": format!("{base_srv}/register")
                            }),
                        );
                    }
                    "/register" => {
                        write_json_http(
                            &mut stream,
                            "201 Created",
                            &json!({ "client_id": "flow_client_1" }),
                        );
                    }
                    "/token" => {
                        let form: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        assert_eq!(form.get("code").map(String::as_str), Some("code_ok_1"));
                        assert_eq!(form.get("client_id").map(String::as_str), Some("flow_client_1"));
                        assert!(!form.get("code_verifier").unwrap().is_empty());
                        write_json_http(
                            &mut stream,
                            "200 OK",
                            &json!({
                                "access_token": "vault_saved_tok",
                                "refresh_token": "vault_saved_ref",
                                "expires_in": 7200
                            }),
                        );
                    }
                    other => panic!("unexpected path: {other}"),
                }
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::open(dir.path().join("vault.json")).unwrap());
        let tokens = run_interactive_oauth_flow(
            "linear",
            &base,
            Arc::clone(&vault),
            |auth_url| {
                let parsed = Url::parse(auth_url).unwrap();
                let params: BTreeMap<String, String> = parsed.query_pairs().into_owned().collect();
                let redirect_uri = params.get("redirect_uri").unwrap().clone();
                let state = params.get("state").unwrap().clone();
                assert_eq!(params.get("code_challenge_method").map(String::as_str), Some("S256"));
                thread::spawn(move || {
                    let client = Client::builder().no_proxy().build().unwrap();
                    let cb = format!("{redirect_uri}?code=code_ok_1&state={state}");
                    let resp = client.get(&cb).send().unwrap();
                    assert_eq!(resp.status().as_u16(), 200);
                });
            },
        )
        .unwrap();

        assert_eq!(tokens.access_token, "vault_saved_tok");
        let mgr = McpOAuthManager::new(Arc::clone(&vault));
        let loaded = mgr.load_tokens("linear").unwrap().unwrap();
        assert_eq!(loaded.access_token, "vault_saved_tok");
        assert_eq!(loaded.refresh_token.as_deref(), Some("vault_saved_ref"));
        assert!(vault.get_secret("mcp_oauth:linear").unwrap().is_some());
        server.join().unwrap();

        // 2. Tampered state must be rejected!
        let bad_state_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bad_addr = bad_state_listener.local_addr().unwrap();
        let bad_base = format!("http://{bad_addr}");
        let bad_base_srv = bad_base.clone();
        let srv2 = thread::spawn(move || {
            for stream in bad_state_listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let (_m, path, _b) = read_raw_http(&mut stream);
                if path == "/.well-known/oauth-protected-resource" {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                } else {
                    write_json_http(
                        &mut stream,
                        "200 OK",
                        &json!({
                            "issuer": bad_base_srv,
                            "authorization_endpoint": format!("{bad_base_srv}/authorize"),
                            "token_endpoint": format!("{bad_base_srv}/token")
                        }),
                    );
                }
            }
        });
        let err = run_interactive_oauth_flow_with_options(
            "badstate",
            &bad_base,
            Arc::clone(&vault),
            Some("pre_client"),
            &[],
            Duration::from_secs(5),
            |auth_url| {
                let parsed = Url::parse(auth_url).unwrap();
                let params: BTreeMap<String, String> = parsed.query_pairs().into_owned().collect();
                let redirect_uri = params.get("redirect_uri").unwrap().clone();
                thread::spawn(move || {
                    let client = Client::builder().no_proxy().build().unwrap();
                    let cb = format!("{redirect_uri}?code=evil&state=tampered_state");
                    let resp = client.get(&cb).send().unwrap();
                    assert_eq!(resp.status().as_u16(), 400);
                });
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("state verification failed"));
        srv2.join().unwrap();

        // 3. Timeout must trigger cleanly
        let to_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let to_addr = to_listener.local_addr().unwrap();
        let to_base = format!("http://{to_addr}");
        let to_base_srv = to_base.clone();
        let srv3 = thread::spawn(move || {
            for stream in to_listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let (_m, path, _b) = read_raw_http(&mut stream);
                if path == "/.well-known/oauth-protected-resource" {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                } else {
                    write_json_http(
                        &mut stream,
                        "200 OK",
                        &json!({
                            "issuer": to_base_srv,
                            "authorization_endpoint": format!("{to_base_srv}/authorize"),
                            "token_endpoint": format!("{to_base_srv}/token")
                        }),
                    );
                }
            }
        });
        let err = run_interactive_oauth_flow_with_options(
            "timeout_srv",
            &to_base,
            Arc::clone(&vault),
            Some("pre_client"),
            &[],
            Duration::from_millis(80),
            |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"));
        srv3.join().unwrap();
    }
}
