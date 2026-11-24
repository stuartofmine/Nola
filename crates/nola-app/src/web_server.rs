// SPDX-License-Identifier: Apache-2.0
//! Secure HTTP + SSE WebHost (`tiny_http`) serving `apps/web/` and all `nola-modules` APIs.
//!
//! Security invariants:
//! - Strict same-origin CSRF validation (`Origin` host == `Host`, `Sec-Fetch-Site` != `cross-site`).
//! - O(1) bounded nonce tracker (`VecDeque` + `HashSet`) preventing replay or memory growth.
//! - Path traversal guard on static asset serving.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_types::new_id;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

const MAX_TRACKED_NONCES: usize = 4_096;

/// O(1) bounded replay-nonce cache (no O(N) linear scan or unbounded memory leak).
#[derive(Default)]
pub struct BoundedNonceCache {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl BoundedNonceCache {
    pub fn record_if_fresh(&mut self, nonce: String) -> bool {
        if self.set.contains(&nonce) {
            return false;
        }
        if self.order.len() >= MAX_TRACKED_NONCES {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        self.set.insert(nonce.clone());
        self.order.push_back(nonce);
        true
    }
}

/// Strict same-origin CSRF check for all mutating HTTP requests.
pub fn validate_same_origin(request: &Request) -> Result<(), &'static str> {
    if matches!(request.method(), Method::Get | Method::Head | Method::Options) {
        return Ok(());
    }
    let host = header_value(request, "host").unwrap_or("");
    if let Some(sfs) = header_value(request, "sec-fetch-site") {
        let lower = sfs.trim().to_ascii_lowercase();
        if !matches!(lower.as_str(), "same-origin" | "none") {
            return Err("rejected cross-site request via Sec-Fetch-Site");
        }
    }
    if let Some(origin) = header_value(request, "origin") {
        let parsed = Url::parse(origin).map_err(|_| "invalid Origin header")?;
        let origin_host = match (parsed.host_str(), parsed.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => return Err("Origin missing host"),
        };
        if !origin_host.eq_ignore_ascii_case(host.trim()) {
            return Err("Origin header does not match Host");
        }
    }
    Ok(())
}

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

pub fn run_web_server(store: Store, suite: Arc<ModulesSuite>, args: &[String]) -> Result<u8, String> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 7860;
    let mut assets_dir = suite.workspace_root.join("apps/web/dist");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                if let Some(h) = args.get(i) {
                    host = h.clone();
                }
            }
            "--port" => {
                i += 1;
                if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) {
                    port = p;
                }
            }
            "--assets" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    assets_dir = PathBuf::from(a);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let addr = format!("{host}:{port}");
    let server = Server::http(&addr).map_err(|e| format!("failed to bind {addr}: {e}"))?;
    let host_id = new_id();
    let nonces = Arc::new(Mutex::new(BoundedNonceCache::default()));
    println!("Nola v2 WebHost listening on http://{addr}");

    for mut req in server.incoming_requests() {
        if let Err(reason) = validate_same_origin(&req) {
            let _ = respond_json(req, 403, &json!({"error": "csrf_blocked", "message": reason}));
            continue;
        }
        if let Some(nonce) = header_value(&req, "x-nola-nonce").map(ToString::to_string) {
            if !nonces.lock().unwrap().record_if_fresh(nonce) {
                let _ = respond_json(req, 409, &json!({"error": "replayed_nonce"}));
                continue;
            }
        }

        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("/");
        let method = req.method().clone();

        match (method, path) {
            (Method::Get, "/api/web/session") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "authenticated": false,
                        "localTrust": false,
                        "transportIngress": "tailcat",
                        "hostId": host_id,
                        "version": "2.0.0"
                    }),
                );
            }
            (Method::Get, "/api/sessions") => {
                let sessions = store.list_sessions(100).unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"sessions": sessions}));
            }
            (Method::Get, "/api/skills") => {
                let skills = nola_modules::skills::discover_skills(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                );
                let _ = respond_json(req, 200, &json!({"skills": skills}));
            }
            (Method::Get, "/api/memory") => {
                let items = store.search_memory("", None, 100).unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"items": items}));
            }
            (Method::Get, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/");
                let items: Vec<Value> = suite.list_records(kind).unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"kind": kind, "items": items}));
            }
            (Method::Post, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/").to_string();
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                match serde_json::from_str::<Value>(&body) {
                    Ok(val) => {
                        let id = val
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        match suite.upsert_record(&kind, &id, &val) {
                            Ok(()) => {
                                let _ = respond_json(req, 200, &json!({"ok": true, "id": id}));
                            }
                            Err(e) => {
                                let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = respond_json(req, 400, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/modules/") => {
                let rest = p.trim_start_matches("/api/modules/");
                if let Some((kind, id)) = rest.split_once('/') {
                    let deleted = suite.delete_record(kind, id).unwrap_or(false);
                    let _ = respond_json(req, 200, &json!({"deleted": deleted}));
                } else {
                    let _ = respond_json(req, 400, &json!({"error": "missing module id"}));
                }
            }
            (Method::Get, p) => {
                serve_static_asset(req, &assets_dir, p);
            }
            _ => {
                let _ = respond_json(req, 404, &json!({"error": "not_found"}));
            }
        }
    }
    Ok(0)
}

fn respond_json(req: Request, status: u16, value: &Value) -> std::io::Result<()> {
    let data = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(header);
    req.respond(resp)
}

fn serve_static_asset(req: Request, assets_dir: &PathBuf, path: &str) {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains("..") {
        let _ = respond_json(req, 403, &json!({"error": "forbidden_path"}));
        return;
    }
    let candidate = assets_dir.join(rel);
    if let Ok(bytes) = fs::read(&candidate) {
        let resp = Response::from_data(bytes);
        let _ = req.respond(resp);
    } else {
        let _ = respond_json(req, 404, &json!({"error": "not_found"}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_nonce_cache_evicts_oldest_in_o1_and_blocks_replays() {
        let mut cache = BoundedNonceCache::default();
        assert!(cache.record_if_fresh("n1".into()));
        assert!(!cache.record_if_fresh("n1".into()));
        for i in 0..MAX_TRACKED_NONCES {
            assert!(cache.record_if_fresh(format!("fill_{i}")));
        }
        // n1 evicted after capacity reached
        assert!(cache.record_if_fresh("n1".into()));
    }
}
