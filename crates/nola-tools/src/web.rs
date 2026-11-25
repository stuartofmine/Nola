// SPDX-License-Identifier: Apache-2.0
//! DNS-pinned SSRF-safe `web_fetch` tool using `reqwest` + `scraper`.
//! Always preserves full HTTP error body/status in `ToolModelPayload::Structured`.

use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use scraper::{Html, Node};
use serde_json::{json, Value};
use url::{Host, Url};

use crate::{ToolContext, ToolHandler};

const MAX_FETCH_BODY_BYTES: u64 = 4 * 1024 * 1024;

pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
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
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Resolve host DNS upfront, reject any private/loopback/link-local/IPv4-mapped address,
/// and return pinned `SocketAddr`s for `reqwest::ClientBuilder::resolve_to_addrs`.
pub fn resolve_and_validate_public_url(raw_url: &str) -> Result<(Url, String, Vec<SocketAddr>), String> {
    let url = Url::parse(raw_url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https schemes are allowed".into());
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let (host, addrs): (String, Vec<SocketAddr>) = match url.host() {
        Some(Host::Ipv4(v4)) => (v4.to_string(), vec![SocketAddr::new(IpAddr::V4(v4), port)]),
        Some(Host::Ipv6(v6)) => (v6.to_string(), vec![SocketAddr::new(IpAddr::V6(v6), port)]),
        Some(Host::Domain(dom)) => {
            let resolved: Vec<SocketAddr> = (dom, port)
                .to_socket_addrs()
                .map_err(|e| format!("dns resolution failed for {dom}: {e}"))?
                .collect();
            (dom.to_string(), resolved)
        }
        None => return Err("url missing host".into()),
    };
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

        // Safely follow up to 5 HTTP redirects by re-validating & DNS-pinning EVERY hop!
        const MAX_REDIRECTS: usize = 5;
        let mut current_url_str = raw_url.to_string();
        let mut resp_opt = None;
        for hop in 0..=MAX_REDIRECTS {
            let (url, host, addrs) = match resolve_and_validate_public_url(&current_url_str) {
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
                        Some(json!({"url": current_url_str})),
                    )
                }
            };
            let status_code = resp.status().as_u16();
            if matches!(status_code, 301 | 302 | 303 | 307 | 308) {
                if hop == MAX_REDIRECTS {
                    return ToolExecutionResult::err_structured(
                        "too_many_redirects",
                        format!("exceeded {MAX_REDIRECTS} redirects fetching {raw_url}"),
                        None,
                    );
                }
                let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) else {
                    resp_opt = Some(resp);
                    break;
                };
                match url.join(loc) {
                    Ok(next) => {
                        current_url_str = next.to_string();
                        continue;
                    }
                    Err(e) => {
                        return ToolExecutionResult::err_structured(
                            "invalid_redirect",
                            format!("invalid redirect Location '{loc}': {e}"),
                            None,
                        );
                    }
                }
            }
            resp_opt = Some(resp);
            break;
        }
        let resp = resp_opt.unwrap();
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let mut buf = Vec::new();
        if let Err(e) = resp.take(MAX_FETCH_BODY_BYTES).read_to_end(&mut buf) {
            return ToolExecutionResult::err_structured(
                "fetch_read_error",
                format!("failed to read HTTP body: {e}"),
                None,
            );
        }
        let body = String::from_utf8_lossy(&buf).into_owned();
        if status >= 400 {
            let preview: String = body.chars().take(2_000).collect();
            return ToolExecutionResult::err_structured(
                "http_error",
                format!("HTTP {status} from {current_url_str}"),
                Some(json!({"status": status, "url": current_url_str, "body": preview})),
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
                "url": current_url_str,
                "status": status,
                "content_type": content_type,
                "truncated": truncated,
                "content": text
            }),
            format!("Fetched {current_url_str} (HTTP {status})"),
        )
    }
}

fn extract_html_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut chunks = Vec::new();
    fn visit(node: scraper::ElementRef<'_>, out: &mut Vec<String>) {
        let tag = node.value().name();
        if matches!(tag, "script" | "style" | "noscript" | "svg") {
            return;
        }
        for child in node.children() {
            match child.value() {
                Node::Text(t) => {
                    let s = t.trim();
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
                Node::Element(_) => {
                    if let Some(el) = scraper::ElementRef::wrap(child) {
                        visit(el, out);
                    }
                }
                _ => {}
            }
        }
    }
    visit(doc.root_element(), &mut chunks);
    chunks.join("\n")
}
