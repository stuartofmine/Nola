// SPDX-License-Identifier: Apache-2.0
//! Nola-owned, local OSS Sign in with ChatGPT. Never reads another app's tokens.
//! Public snapshots are secret-free; this module is not a generic API-key provider.
use crate::host_credentials::HostCredentialVault;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::{File, OpenOptions}, io::{BufRead, BufReader, Read, Write}, net::TcpListener, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const IDENTITY: &str = "nola-owned-siwc-v1";
const MAX_JSON: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
struct Endpoints { issuer: String, resource: String }

pub struct SiwcHost { database: PathBuf, endpoints: Endpoints, http: Client }

#[derive(Clone, Debug)]
pub struct SiwcExecutionContext {
    pub database: PathBuf,
    pub account: nola_host_protocol::HostSiwcAccountBinding,
}

/// Not Clone/Debug/Serialize. The lock prevents sign-out racing an active stream.
pub struct SiwcExecutionCredential {
    bearer: String,
    resource: String,
    _lock: SessionLock,
}
impl SiwcExecutionCredential {
    pub(crate) fn bearer(&self) -> &str { &self.bearer }
    pub(crate) fn resource_url(&self) -> &str { &self.resource }
}

#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    id_token: String,
    scopes: Vec<String>,
    expires_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    host_id: String,
    issuer: String,
    client_id: String,
    subject: String,
    email: Option<String>,
    tokens: Option<Tokens>,
    /// Persisted before using a rotating refresh token. Unknown outcomes require reauthorization.
    refresh_in_flight: bool,
    revocation_unconfirmed: bool,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub account_id: String,
    pub session_revision: i64,
    pub client_id: String,
    pub subject: String,
    pub email: Option<String>,
    pub signed_in: bool,
    pub plan_usage_authorized: bool,
    pub reauthorization_required: bool,
    pub revocation_unconfirmed: bool,
}

/// Intentionally not Debug/Serialize/Clone: contains one-use PKCE and a live listener.
pub struct AuthorizationAttempt {
    listener: TcpListener,
    account_id: String,
    expected_revision: i64,
    host_id: String,
    previous: Option<Session>,
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    deadline: Instant,
    authorization_url: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct AuthorizationCancellation(Arc<AtomicBool>);
impl AuthorizationCancellation {
    pub fn cancel(&self) { self.0.store(true, Ordering::SeqCst); }
}

impl AuthorizationAttempt {
    /// Send only to the explicitly initiated system-browser launch; never log this URL.
    pub fn authorization_url(&self) -> &str { &self.authorization_url }
    pub fn cancellation(&self) -> AuthorizationCancellation { AuthorizationCancellation(self.cancelled.clone()) }
    pub fn cancel(self) { /* Dropping the listener and pending secrets cancels locally. */ }
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    revocation_endpoint: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    id_token: Option<String>,
    token_type: String,
    #[serde(default)]
    expires_in: i64,
    scope: String,
}

#[derive(Clone, Deserialize)]
struct Claims { sub: String, email: Option<String>, nonce: Option<String>, iat: i64, exp: i64, azp: Option<String>, aud: Value }

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ModelChoice {
    pub slug: String,
    pub display_name: String,
    /// Exact optional metadata from the selected account's live catalog.
    #[serde(default)]
    pub supported_reasoning_levels: Vec<CatalogReasoningLevel>,
    #[serde(default)]
    pub supports_reasoning_summary_parameter: Option<bool>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogReasoningLevel { pub effort: String }


#[derive(Debug, Serialize)]
pub struct InferenceOutcome {
    pub state: &'static str,
    pub response_id: Option<String>,
    pub usage: Option<Value>,
}

/// Native file locks cover multiple Host processes on Unix and Windows.
struct SessionLock(File);
impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl SiwcHost {
    /// Keep private-session rotation/sign-out serialized through a native Core CAS.
    pub(crate) fn with_session_guard<T>(&self, expected: &Snapshot, commit: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let _lock = self.lock(&expected.account_id)?;
        let current = self.snapshot(&expected.account_id)?.ok_or("native_siwc_private_session_changed")?;
        if serde_json::to_value(&current).ok() != serde_json::to_value(expected).ok() {
            return Err("native_siwc_private_session_changed".into());
        }
        commit()
    }

    pub fn open(database: &Path) -> Result<Self, String> {
        let http = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(120))
            .build().map_err(|_| "siwc_transport_unavailable")?;
        let database = std::fs::canonicalize(database).map_err(|_| "siwc_database_unavailable")?;
        Ok(Self { database, endpoints: Endpoints { issuer: ISSUER.into(), resource: RESOURCE.into() }, http })
    }

    fn lock(&self, account: &str) -> Result<SessionLock, String> {
        let digest = hex::encode(Sha256::digest(account.as_bytes()));
        let path = self.database.with_extension(format!("siwc-{digest}.lock"));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)] {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = options.open(path).map_err(|_| "siwc_lock_unavailable")?;
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt;
            if file.metadata().map_err(|_| "siwc_lock_unavailable")?.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("siwc_lock_unavailable".into());
            }
        }
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => "siwc_account_busy",
            std::fs::TryLockError::Error(_) => "siwc_lock_unavailable",
        })?;
        Ok(SessionLock(file))
    }

    fn slot(account: &str) -> Result<String, String> {
        let id = uuid::Uuid::parse_str(account).map_err(|_| "siwc_account_id_invalid")?;
        if id.to_string() != account { return Err("siwc_account_id_invalid".into()); }
        Ok(format!("nola-siwc-{account}"))
    }

    fn read(&self, account: &str) -> Result<(i64, Option<Session>), String> {
        let record = HostCredentialVault::open(&self.database)?.record(&Self::slot(account)?)?;
        match record {
            None => Ok((0, None)),
            Some(record) if record.identity == IDENTITY => {
                let session = record.secret.as_deref().map(serde_json::from_str).transpose().map_err(|_| "siwc_session_invalid")?;
                Ok((record.revision, session))
            }
            _ => Err("siwc_owner_mismatch".into()),
        }
    }

    fn write(&self, account: &str, revision: i64, session: &Session) -> Result<i64, String> {
        let encoded = serde_json::to_string(session).map_err(|_| "siwc_session_invalid")?;
        HostCredentialVault::open(&self.database)?.write(&Self::slot(account)?, IDENTITY, revision, Some(&encoded), || Ok(()))
    }

    fn host_id(&self) -> Result<String, String> {
        let _lock = self.lock("host-id")?;
        let mut vault = HostCredentialVault::open(&self.database)?;
        if let Some(record) = vault.record("nola-siwc-host")? {
            if record.identity != IDENTITY { return Err("siwc_host_invalid".into()); }
            let host = record.secret.ok_or("siwc_host_invalid")?;
            uuid::Uuid::parse_str(host.strip_prefix("urn:uuid:").ok_or("siwc_host_invalid")?).map_err(|_| "siwc_host_invalid")?;
            return Ok(host);
        }
        let host = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        vault.write("nola-siwc-host", IDENTITY, 0, Some(&host), || Ok(()))?;
        Ok(host)
    }

    fn discovery(&self) -> Result<Discovery, String> {
        let d: Discovery = self.get_json(&format!("{}/.well-known/openid-configuration", self.endpoints.issuer), None)?;
        if d.issuer != self.endpoints.issuer { return Err("siwc_issuer_mismatch".into()); }
        for endpoint in [&d.authorization_endpoint, &d.token_endpoint, &d.jwks_uri, &d.revocation_endpoint] {
            let url = reqwest::Url::parse(endpoint).map_err(|_| "siwc_discovery_invalid")?;
            let issuer = reqwest::Url::parse(&self.endpoints.issuer).map_err(|_| "siwc_discovery_invalid")?;
            if url.origin() != issuer.origin() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
                return Err("siwc_discovery_invalid".into());
            }
        }
        Ok(d)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, bearer: Option<&str>) -> Result<T, String> {
        let mut request = self.http.get(url);
        if let Some(token) = bearer { request = request.bearer_auth(token); }
        read_json(request.send().map_err(|_| "siwc_network_unavailable")?)
    }

    pub fn snapshot(&self, account: &str) -> Result<Option<Snapshot>, String> {
        let (revision, session) = self.read(account)?;
        Ok(session.map(|s| snapshot(account, revision, &s)))
    }

    /// Explicit local OSS sign-in only. Caller must separately launch the returned URL.
    pub fn begin_sign_in(&self, account: &str, expected_revision: i64) -> Result<AuthorizationAttempt, String> {
        let _lock = self.lock(account)?;
        let (revision, previous) = self.read(account)?;
        if revision != expected_revision { return Err("siwc_revision_conflict".into()); }
        let host_id = self.host_id()?;
        let discovery = self.discovery()?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| "siwc_callback_unavailable")?;
        listener.set_nonblocking(true).map_err(|_| "siwc_callback_unavailable")?;
        let redirect_uri = format!("http://127.0.0.1:{}/auth/callback", listener.local_addr().map_err(|_| "siwc_callback_unavailable")?.port());
        let state = random_value(); let nonce = random_value(); let verifier = random_value();
        let mut url = reqwest::Url::parse(&discovery.authorization_endpoint).map_err(|_| "siwc_discovery_invalid")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", previous.as_ref().map(|s| s.client_id.as_str()).unwrap_or(DYNAMIC_CLIENT))
                .append_pair("ext_agent_host_id", &host_id).append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri).append_pair("scope", SCOPES)
                .append_pair("resource", &self.endpoints.resource).append_pair("state", &state)
                .append_pair("nonce", &nonce).append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())));
            if previous.is_none() { query.append_pair("agent_name_hint", "Nola"); }
            // No ID-token hint: avoid sending retained tokens through UI-visible URLs.
        }
        Ok(AuthorizationAttempt { listener, account_id:account.into(), expected_revision, host_id,
            previous, state, nonce, verifier, redirect_uri, deadline:Instant::now()+Duration::from_secs(600), authorization_url:url.into(), cancelled:Arc::new(AtomicBool::new(false)) })
    }

    /// One callback only; no UI-supplied token or unverified decoded identity accepted.
    pub fn finish_sign_in(&self, attempt: AuthorizationAttempt) -> Result<Snapshot, String> {
        let mut connection = loop {
            if attempt.cancelled.load(Ordering::SeqCst) { return Err("siwc_cancelled".into()); }
            if Instant::now() >= attempt.deadline { return Err("siwc_callback_expired".into()); }
            match attempt.listener.accept() {
                Ok((stream, peer)) if peer.ip().is_loopback() => break stream,
                Ok(_) => return Err("siwc_callback_invalid".into()),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => return Err("siwc_callback_unavailable".into()),
            }
        };
        connection.set_read_timeout(Some(Duration::from_secs(5))).map_err(|_| "siwc_callback_unavailable")?;
        connection.set_write_timeout(Some(Duration::from_secs(5))).map_err(|_| "siwc_callback_unavailable")?;
        let mut line = String::new();
        BufReader::new((&connection).take(8193)).read_line(&mut line).map_err(|_| "siwc_callback_invalid")?;
        let result = if line.len() > 8192 { Err("siwc_callback_invalid".into()) } else {
            let parts: Vec<_> = line.trim_end().split(' ').collect();
            if parts.len()!=3 || parts[0]!="GET" || parts[2]!="HTTP/1.1" { Err("siwc_callback_invalid".into()) }
            else { self.exchange_callback(&attempt, parts[1]) }
        };
        let body = if result.is_ok() { "Nola sign-in completed. Return to Nola." } else { "Nola sign-in could not be completed. Return to Nola." };
        let _ = write!(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        result
    }

    fn exchange_callback(&self, attempt: &AuthorizationAttempt, path: &str) -> Result<Snapshot, String> {
        if attempt.cancelled.load(Ordering::SeqCst) { return Err("siwc_cancelled".into()); }
        if Instant::now() >= attempt.deadline || !path.starts_with("/auth/callback?") { return Err("siwc_callback_invalid".into()); }
        let callback = reqwest::Url::parse(&format!("http://127.0.0.1{path}")).map_err(|_| "siwc_callback_invalid")?;
        if callback.path() != "/auth/callback" || callback.fragment().is_some() { return Err("siwc_callback_invalid".into()); }
        let mut values = BTreeMap::new();
        for (key,value) in callback.query_pairs() {
            if values.insert(key.into_owned(),value.into_owned()).is_some() { return Err("siwc_callback_duplicate_field".into()); }
        }
        if values.get("state") != Some(&attempt.state) { return Err("siwc_state_mismatch".into()); }
        if values.contains_key("error") { return Err("siwc_authorization_denied".into()); }
        let client_id = match (&attempt.previous, values.get("client_id")) {
            (Some(previous), supplied) if supplied.is_none_or(|id| id == &previous.client_id) => previous.client_id.clone(),
            (None, Some(id)) if !id.is_empty() && id != DYNAMIC_CLIENT && id.len() <= 512 => id.clone(),
            _ => return Err("siwc_client_mismatch".into()),
        };
        let code = values.get("code").filter(|s| !s.is_empty()).ok_or("siwc_code_missing")?;
        let _lock = self.lock(&attempt.account_id)?;
        if self.read(&attempt.account_id)?.0 != attempt.expected_revision { return Err("siwc_revision_conflict".into()); }
        let discovery = self.discovery()?;
        let response = self.http.post(&discovery.token_endpoint).form(&[
            ("grant_type","authorization_code"),("client_id",client_id.as_str()),("code",code),
            ("code_verifier",attempt.verifier.as_str()),("redirect_uri",attempt.redirect_uri.as_str()),("resource",self.endpoints.resource.as_str())
        ]).send().map_err(|_| "siwc_exchange_result_unknown")?;
        let tokens: TokenResponse = read_json(response)?;
        let id_token = tokens.id_token.as_deref().ok_or("siwc_identity_missing")?;
        let claims = self.verify_identity(&discovery, id_token, &client_id, Some(&attempt.nonce))?;
        if attempt.previous.as_ref().is_some_and(|s| s.subject != claims.sub || s.issuer != discovery.issuer || s.client_id != client_id) { return Err("siwc_identity_mismatch".into()); }
        let tokens = token_set(tokens, None)?;
        if attempt.cancelled.load(Ordering::SeqCst) { return Err("siwc_cancelled".into()); }
        let session = Session { host_id:attempt.host_id.clone(), issuer:discovery.issuer, client_id,
            subject:claims.sub, email:claims.email, tokens:Some(tokens), refresh_in_flight:false, revocation_unconfirmed:false };
        let revision = self.write(&attempt.account_id, attempt.expected_revision, &session)?;
        Ok(snapshot(&attempt.account_id, revision, &session))
    }

    fn verify_identity(&self, discovery: &Discovery, token: &str, client: &str, nonce: Option<&str>) -> Result<Claims, String> {
        let header = decode_header(token).map_err(|_| "siwc_identity_invalid")?;
        if header.alg != Algorithm::RS256 { return Err("siwc_identity_algorithm_rejected".into()); }
        let kid = header.kid.ok_or("siwc_identity_key_missing")?;
        let keys: JwkSet = self.get_json(&discovery.jwks_uri,None)?;
        let key = keys.find(&kid).ok_or("siwc_identity_key_unknown")?;
        let key = DecodingKey::from_jwk(key).map_err(|_| "siwc_identity_key_invalid")?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[discovery.issuer.as_str()]); validation.set_audience(&[client]);
        validation.set_required_spec_claims(&["sub","exp","iat","iss","aud"]);
        validation.leeway = 5; validation.validate_nbf = true;
        let claims = decode::<Claims>(token,&key,&validation).map_err(|_| "siwc_identity_invalid")?.claims;
        let now = chrono::Utc::now().timestamp();
        if claims.sub.is_empty() || claims.iat > now+5 || claims.exp <= claims.iat
            || nonce.is_some_and(|value| claims.nonce.as_deref()!=Some(value))
            || claims.azp.as_deref().is_some_and(|value| value != client)
            || (claims.aud.as_array().is_some_and(|values| values.len()>1) && claims.azp.as_deref()!=Some(client)) { return Err("siwc_identity_invalid".into()); }
        Ok(claims)
    }

    pub fn refresh(&self, account: &str, expected_revision: i64) -> Result<Snapshot, String> {
        let _lock = self.lock(account)?;
        self.refresh_locked(account, expected_revision)
    }

    fn refresh_locked(&self, account: &str, expected_revision: i64) -> Result<Snapshot, String> {
        let (revision, session) = self.read(account)?;
        if revision != expected_revision { return Err("siwc_revision_conflict".into()); }
        let mut session = session.ok_or("siwc_sign_in_required")?;
        if session.refresh_in_flight || session.revocation_unconfirmed { return Err("siwc_reauthorization_required".into()); }
        let previous = session.tokens.clone().ok_or("siwc_sign_in_required")?;
        if previous.refresh_token.is_empty() { return Err("siwc_reauthorization_required".into()); }
        let discovery = self.discovery()?;
        session.refresh_in_flight = true;
        let revision = self.write(account,revision,&session)?;
        let response = self.http.post(&discovery.token_endpoint).form(&[
            ("grant_type","refresh_token"),("client_id",session.client_id.as_str()),
            ("refresh_token",previous.refresh_token.as_str()),("resource",self.endpoints.resource.as_str())
        ]).send().map_err(|_| "siwc_refresh_result_unknown")?;
        let tokens: TokenResponse = read_json(response)?;
        if let Some(id_token) = &tokens.id_token {
            let claims = self.verify_identity(&discovery,id_token,&session.client_id,None)?;
            if claims.sub != session.subject { return Err("siwc_identity_mismatch".into()); }
        }
        session.tokens = Some(token_set(tokens,Some(&previous.id_token))?);
        session.refresh_in_flight = false;
        let revision = self.write(account,revision,&session)?;
        Ok(snapshot(account,revision,&session))
    }

    fn require_core_owner(&self, account_id:&str, account_revision:i64, credential_ref:&str) -> Result<(),String> {
        let store=crate::store::NolaStore::open(&self.database)?;
        let account:Option<nola_domain::model_connection::ModelAccountSnapshot>=store.application()?.execute(nola_application::ApplicationCommand::Model {
            command:nola_application::ModelCommand::GetAccount {id:account_id.into()}
        })?;
        let account=account.ok_or("siwc_core_account_unavailable")?;
        if account.revision!=account_revision || account.account.state!=nola_domain::model_connection::ModelAccountState::Ready
            || serde_json::to_value(account.account.source).map_err(|_|"siwc_core_owner_invalid")?!="nola_chatgpt"
            || account.account.credential_ref.as_deref()!=Some(credential_ref)
            || credential_ref!=format!("siwc:{account_id}") {return Err("siwc_core_owner_changed".into());}
        Ok(())
    }

    /// Trusted admission freezes identity, never an access token or rotating vault revision.
    pub fn execution_binding(&self, account_id:&str, account_revision:i64, credential_ref:&str) -> Result<nola_host_protocol::HostSiwcAccountBinding,String> {
        self.require_core_owner(account_id,account_revision,credential_ref)?;
        let (_,session)=self.read(account_id)?;
        let session=session.ok_or("siwc_sign_in_required")?;
        // Admission may capture an expired but renewable session. Dispatch performs refresh.
        if session.refresh_in_flight || session.revocation_unconfirmed || !session.tokens.as_ref().is_some_and(|t|plan_scopes(&t.scopes)&&!t.access_token.is_empty()) {
            return Err("siwc_reauthorization_required".into());
        }
        Ok(nola_host_protocol::HostSiwcAccountBinding {account_id:account_id.into(),account_revision,credential_ref:credential_ref.into(),client_id:session.client_id,subject:session.subject})
    }

    /// Only the dedicated Responses adapter may consume this bearer. No API-key fallback.
    pub fn resolve_execution_credential(&self, binding:&nola_host_protocol::HostSiwcAccountBinding) -> Result<SiwcExecutionCredential,String> {
        let lock=self.lock(&binding.account_id)?;
        self.require_core_owner(&binding.account_id,binding.account_revision,&binding.credential_ref)?;
        let (revision,session)=self.read(&binding.account_id)?;
        let session=session.ok_or("siwc_sign_in_required")?;
        if session.issuer!=self.endpoints.issuer || session.client_id!=binding.client_id || session.subject!=binding.subject {
            return Err("siwc_execution_identity_changed".into());
        }
        if session.tokens.as_ref().is_some_and(|t|t.expires_at<=chrono::Utc::now().timestamp()+30) {
            self.refresh_locked(&binding.account_id,revision)?;
        }
        self.require_core_owner(&binding.account_id,binding.account_revision,&binding.credential_ref)?;
        let (revision,_)=self.read(&binding.account_id)?;
        let session=self.authorized(&binding.account_id,revision)?;
        if session.client_id!=binding.client_id || session.subject!=binding.subject {return Err("siwc_execution_identity_changed".into());}
        Ok(SiwcExecutionCredential {bearer:session.tokens.ok_or("siwc_sign_in_required")?.access_token,resource:self.endpoints.resource.clone(),_lock:lock})
    }

    /// Clear only this registration. An unknown remote result is explicit in the receipt.
    pub fn sign_out(&self, account: &str, expected_revision: i64) -> Result<Snapshot, String> {
        let _lock = self.lock(account)?;
        let (revision, session) = self.read(account)?;
        if revision != expected_revision { return Err("siwc_revision_conflict".into()); }
        let mut session = session.ok_or("siwc_sign_in_required")?;
        let tokens = session.tokens.clone();
        session.revocation_unconfirmed |= tokens.as_ref().is_some_and(|t| !t.refresh_token.is_empty());
        let revision = self.write(account,revision,&session)?; // Blocks inference before network.
        let revoked = if let Some(tokens) = tokens.filter(|t| !t.refresh_token.is_empty()) {
            self.discovery().and_then(|d| self.http.post(d.revocation_endpoint).form(&[
                ("token",tokens.refresh_token.as_str()),("token_type_hint","refresh_token"),("client_id",session.client_id.as_str())
            ]).send().map_err(|_| "siwc_revocation_unconfirmed".into()))
                .is_ok_and(|r| r.status().as_u16()==200)
        } else { !session.revocation_unconfirmed };
        session.tokens = None; session.refresh_in_flight = false; session.revocation_unconfirmed = !revoked;
        let revision = self.write(account,revision,&session)?;
        Ok(snapshot(account,revision,&session))
    }

    fn authorized(&self, account: &str, revision: i64) -> Result<Session, String> {
        let (current,session) = self.read(account)?;
        if current != revision { return Err("siwc_revision_conflict".into()); }
        let session = session.ok_or("siwc_sign_in_required")?;
        if session.refresh_in_flight || session.revocation_unconfirmed { return Err("siwc_reauthorization_required".into()); }
        let tokens = session.tokens.as_ref().ok_or("siwc_sign_in_required")?;
        if tokens.expires_at <= chrono::Utc::now().timestamp()+30 { return Err("siwc_refresh_required".into()); }
        if !plan_scopes(&tokens.scopes) || tokens.access_token.is_empty() { return Err("siwc_plan_consent_required".into()); }
        Ok(session)
    }

    pub fn models(&self, account: &str, revision: i64) -> Result<Vec<ModelChoice>, String> {
        let _lock = self.lock(account)?;
        let (current_revision, session) = self.read(account)?;
        if current_revision != revision { return Err("siwc_revision_conflict".into()); }
        if session.as_ref().and_then(|s| s.tokens.as_ref())
            .is_some_and(|t| t.expires_at <= chrono::Utc::now().timestamp() + 30) {
            self.refresh_locked(account, revision)?;
        }
        let (current_revision, _) = self.read(account)?;
        let session = self.authorized(account,current_revision)?;
        let value: Value = self.get_json(&format!("{}/models",self.endpoints.resource),Some(&session.tokens.as_ref().ok_or("siwc_sign_in_required")?.access_token))?;
        value.get("models").and_then(Value::as_array).ok_or("siwc_catalog_invalid")?.iter()
            .filter(|item| item["visibility"] == "list")
            .map(|item| serde_json::from_value(item.clone()).map_err(|_| "siwc_catalog_invalid".into())).collect()
    }

    /// Text-only vertical slice. Caller owns canonical attempt/admission; no retries or tool execution.
    /// Holding the account lock through the stream linearizes sign-out with active requests.
    #[cfg(test)]
    fn infer_text(&self, account: &str, revision: i64, model: &str, messages: &[Value], mut event: impl FnMut(&Value) -> Result<(), String>) -> Result<InferenceOutcome,String> {
        if model.is_empty() || messages.is_empty() || messages.iter().any(|m| {
            let Some(fields)=m.as_object() else { return true; };
            fields.keys().any(|k| !matches!(k.as_str(),"role"|"content"))
                || !matches!(m["role"].as_str(),Some("user"|"assistant"|"developer")) || !m["content"].is_string()
        }) { return Err("siwc_text_input_invalid".into()); }
        let _lock = self.lock(account)?;
        let session = self.authorized(account,revision)?;
        let token = &session.tokens.as_ref().ok_or("siwc_sign_in_required")?.access_token;
        let response = self.http.post(format!("{}/responses",self.endpoints.resource)).bearer_auth(token)
            .json(&json!({"model":model,"input":messages,"stream":true,"store":false}))
            .send().map_err(|_| "siwc_inference_result_unknown")?;
        if !response.status().is_success() { return Err(format!("siwc_inference_http_{}",response.status().as_u16())); }
        if !response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream")) {
            return Err("siwc_stream_invalid".into());
        }
        consume_stream(response,&mut event)
    }
}

fn random_value() -> String {
    // Two independent OS-random UUIDs supply > 240 random bits, and a 43-char verifier.
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes()); bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}
fn read_json<T: serde::de::DeserializeOwned>(response: Response) -> Result<T,String> {
    if !response.status().is_success() { return Err(format!("siwc_http_{}",response.status().as_u16())); }
    let mut bytes = vec![];
    response.take(MAX_JSON+1).read_to_end(&mut bytes).map_err(|_| "siwc_response_unavailable")?;
    if bytes.len() as u64 > MAX_JSON { return Err("siwc_response_too_large".into()); }
    serde_json::from_slice(&bytes).map_err(|_| "siwc_response_invalid".into())
}
fn token_set(response: TokenResponse, retained_id: Option<&str>) -> Result<Tokens,String> {
    let scopes:Vec<_> = response.scope.split_ascii_whitespace().map(str::to_owned).collect();
    if !response.token_type.eq_ignore_ascii_case("Bearer") || response.expires_in<0 || response.expires_in>86400
        || (plan_scopes(&scopes) && (response.access_token.is_empty() || response.expires_in==0))
        || (scopes.iter().any(|s| s=="offline_access") && response.refresh_token.is_empty()) { return Err("siwc_tokens_invalid".into()); }
    let id_token = response.id_token.or_else(|| retained_id.map(str::to_owned)).ok_or("siwc_identity_missing")?;
    Ok(Tokens { access_token:response.access_token, refresh_token:response.refresh_token,id_token,scopes,
        expires_at:chrono::Utc::now().timestamp().checked_add(response.expires_in).ok_or("siwc_expiry_invalid")? })
}
fn plan_scopes(scopes: &[String]) -> bool {
    ["openid","resource.invoke","chatgpt.tokens.use.direct"].iter().all(|scope| scopes.iter().any(|s| s==scope))
}
fn snapshot(account: &str, revision:i64, s:&Session) -> Snapshot {
    Snapshot { account_id:account.into(),session_revision:revision,client_id:s.client_id.clone(),subject:s.subject.clone(),email:s.email.clone(),
        signed_in:s.tokens.is_some(),plan_usage_authorized:s.tokens.as_ref().is_some_and(|t| plan_scopes(&t.scopes) && !t.access_token.is_empty()) && !s.refresh_in_flight && !s.revocation_unconfirmed,
        reauthorization_required:s.refresh_in_flight,revocation_unconfirmed:s.revocation_unconfirmed }
}

fn consume_stream(reader: impl Read, event: &mut impl FnMut(&Value)->Result<(),String>) -> Result<InferenceOutcome,String> {
    let mut reader=BufReader::new(reader); let mut total=0usize; let mut data=String::new();
    loop {
        let mut line=String::new();
        let count=(&mut reader).take(MAX_JSON+1).read_line(&mut line).map_err(|_| "siwc_stream_interrupted")?;
        total=total.checked_add(count).ok_or("siwc_stream_too_large")?;
        if count as u64>MAX_JSON || total>32*1024*1024 { return Err("siwc_stream_too_large".into()); }
        if count==0 { return Err("siwc_stream_interrupted".into()); }
        if line.trim_end().is_empty() && !data.is_empty() {
            let value:Value=serde_json::from_str(&data).map_err(|_| "siwc_stream_invalid")?; data.clear();
            event(&value)?;
            let state=match value["type"].as_str() { Some("response.completed")=>Some("completed"),Some("response.failed"|"error")=>Some("failed"),Some("response.incomplete")=>Some("incomplete"),_=>None };
            if let Some(state)=state { return Ok(InferenceOutcome {state,response_id:value["response"]["id"].as_str().map(str::to_owned),usage:value["response"].get("usage").filter(|v| !v.is_null()).cloned()}); }
        } else if let Some(value)=line.strip_prefix("data:") {
            if !data.is_empty() { data.push('\n'); } data.push_str(value.trim());
            if data.len() as u64>MAX_JSON { return Err("siwc_stream_too_large".into()); }
        }
    }
}

#[cfg(all(test, any(unix, windows)))]
#[path = "model_siwc_tests.rs"]
mod tests;

#[cfg(test)]
mod lock_tests {
    use super::*;

#[test]
fn account_lock_rejects_a_competing_host_and_releases_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("owner.db");
    File::create(&database).unwrap();
    let first = SiwcHost::open(&database).unwrap();
    let second = SiwcHost::open(&database).unwrap();
    let account = uuid::Uuid::new_v4().to_string();
    let lock = first.lock(&account).unwrap();
    assert_eq!(second.lock(&account).err().as_deref(), Some("siwc_account_busy"));
    drop(lock);
    drop(second.lock(&account).expect("released native lock"));
}

}

#[cfg(all(test, not(any(unix, windows))))]
mod unsupported_vault_tests {
    use super::*;
    #[test]
    fn unavailable_private_storage_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("owner.db");
        File::create(&database).unwrap();
        let host = SiwcHost::open(&database).unwrap();
        let error = host.snapshot(&uuid::Uuid::new_v4().to_string()).err().unwrap();
        assert!(error.contains("protected Unix file permissions"));
        assert!(!directory.path().join(".nola-host-secrets").exists());
    }
}
