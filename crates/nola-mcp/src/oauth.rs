// SPDX-License-Identifier: Apache-2.0
//! MCP OAuth 2.1 PKCE + single-flight token refresh wired directly to `nola-store::Vault`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nola_store::Vault;
use nola_types::now_ms;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::McpError;

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

    pub fn store_tokens(&self, server_id: &str, tokens: &OAuthTokenSet) -> Result<(), McpError> {
        let raw = serde_json::to_string(tokens)?;
        self.vault
            .set_secret(&Self::vault_key(server_id), &raw)
            .map_err(|e| McpError::OAuth(e.to_string()))
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
        let server_lock = {
            let mut locks = self.refresh_locks.lock().unwrap();
            Arc::clone(
                locks
                    .entry(server_id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
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
        self.store_tokens(server_id, &refreshed)?;
        Ok(Some(refreshed.access_token))
    }

    fn refresh_access_token(
        &self,
        existing: &OAuthTokenSet,
        refresh_token: &str,
    ) -> Result<OAuthTokenSet, McpError> {
        let request_started_ms = now_ms();
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| McpError::OAuth(e.to_string()))?;
        let resp = client
            .post(&existing.token_endpoint)
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
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| McpError::OAuth(format!("invalid token response json: {e}")))?;
        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| McpError::OAuth("refresh response missing non-empty access_token".into()))?
            .to_string();
        let new_refresh = body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
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
}
