// SPDX-License-Identifier: Apache-2.0
//! DNS-pinned SSRF-safe `web_fetch` tool using `reqwest` + `scraper`.
//! Always preserves full HTTP error body/status in `ToolModelPayload::Structured`.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use scraper::{Html, Selector};
use serde_json::{json, Value};
use url::Url;

use crate::{ToolContext, ToolHandler};

pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64 // CGNAT 100.64.0.0/10
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Resolve host DNS upfront, reject any private/loopback/link-local address (SSRF guard),
/// and return pinned `SocketAddr`s for `reqwest::ClientBuilder::resolve_to_addrs`.
pub fn resolve_and_validate_public_url(raw_url: &str) -> Result<(Url, String, Vec<SocketAddr>), String> {
    let url = Url::parse(raw_url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https schemes are allowed".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "url missing host".to_string())?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("dns resolution failed for {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("dns returned no addresses for {host}"));
    }
    for addr in &addrs {
        if is_forbidden_ip(addr.ip()) {
            return Err(format!(
                "blocked SSRF target: {host} resolved to non-public address {}",
                addr.ip()
            ));
        }
    }
    Ok((url, host, addrs))
}

pub struct WebFetchTool;

impl ToolHandler for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "web_fetch".into(),
                description: "Fetch a public HTTP/HTTPS URL with DNS-pinned SSRF protection and extract readable text.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string" },
                        "max_chars": { "type": "integer", "description": "Maximum response chars (default 20000)" }
                    },
                    "required": ["url"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Network,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if !ctx.network_allowed {
            return ToolExecutionResult::err_structured(
                "network_denied",
                "network access is disabled by policy",
                None,
            );
        }
        let Some(raw_url) = args.get("url").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'url'", None);
        };
        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(20_000)
            .clamp(256, 200_000) as usize;

        let (url, host, addrs) = match resolve_and_validate_public_url(raw_url) {
            Ok(v) => v,
            Err(e) => return ToolExecutionResult::err_structured("ssrf_blocked", e, None),
        };

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&host, &addrs)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "http_client_error",
                    e.to_string(),
                    None,
                )
            }
        };

        let resp = match client.get(url.clone()).send() {
            Ok(r) => r,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "fetch_error",
                    format!("HTTP request failed: {e}"),
                    Some(json!({"url": raw_url})),
                )
            }
        };
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().unwrap_or_default();
        if status >= 400 {
            let preview: String = body.chars().take(2_000).collect();
            return ToolExecutionResult::err_structured(
                "http_error",
                format!("HTTP {status} from {raw_url}"),
                Some(json!({"status": status, "url": raw_url, "body": preview})),
            );
        }

        let extracted = if content_type.to_ascii_lowercase().contains("text/html") {
            extract_html_text(&body)
        } else {
            body
        };
        let truncated = extracted.chars().count() > max_chars;
        let text: String = extracted.chars().take(max_chars).collect();

        ToolExecutionResult::ok_structured(
            json!({
                "url": raw_url,
                "status": status,
                "content_type": content_type,
                "truncated": truncated,
                "content": text
            }),
            format!("Fetched {raw_url} (HTTP {status})"),
        )
    }
}

fn extract_html_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("body").unwrap();
    let root = doc.select(&sel).next();
    let chunks: Vec<String> = match root {
        Some(el) => el
            .text()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        None => doc
            .root_element()
            .text()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
    };
    chunks.join("\n")
}
