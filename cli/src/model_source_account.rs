// SPDX-License-Identifier: Apache-2.0
//! Read-only local login references. Presence/expiry is not proof that a
//! subscription authorizes Nola API inference. No login, refresh or token copy.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use base64::Engine;
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{
    ModelAccount, ModelAccountSnapshot, ModelAccountState, ModelSourceKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REFERENCE_IDENTITY: &str = "local-source-account-reference-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAccountRequest {
    pub account_id: String,
    pub display_name: String,
    pub source: ModelSourceKind,
    pub path: String,
    pub expected_revision: Option<i64>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceLoginState {
    LoginRequired,
    Expired,
    PresentButUnsupported,
    Invalid,
    Unavailable,
    Disconnected,
}
#[derive(Serialize)]
pub struct SourceAccountStatus {
    pub account: ModelAccountSnapshot,
    pub login_state: SourceLoginState,
    pub inference_supported: bool,
    pub user_action: Option<String>,
    pub expires_at_ms: Option<i64>,
}
#[derive(Serialize, Deserialize)]
struct PrivateReference {
    source: ModelSourceKind,
    path: String,
}

fn expiry_from_jwt(token: &str) -> Option<i64> {
    let part = token.split('.').nth(1)?;
    if part.len() > 64 * 1024 {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part)
        .ok()?;
    let payload: Value = serde_json::from_slice(&bytes).ok()?;
    // Unverified metadata is used only to reject expired material, never to
    // authorize a request or trust an account identity from a JWT.
    payload["exp"].as_i64()?.checked_mul(1000)
}

fn classify(
    source: ModelSourceKind,
    document: &Value,
    now: i64,
) -> (SourceLoginState, Option<i64>) {
    let (token, expiry) = match source {
        ModelSourceKind::Codex => {
            let token = document["tokens"]["access_token"]
                .as_str()
                .filter(|v| !v.is_empty());
            (token, token.and_then(expiry_from_jwt))
        }
        ModelSourceKind::KimiCode => (
            document["access_token"].as_str().filter(|v| !v.is_empty()),
            document["expires_at"]
                .as_i64()
                .and_then(|v| v.checked_mul(1000)),
        ),
        _ => return (SourceLoginState::Unavailable, None),
    };
    if token.is_none() {
        return (SourceLoginState::LoginRequired, None);
    }
    if expiry.is_some_and(|expires| expires <= now) {
        return (SourceLoginState::Expired, expiry);
    }
    (SourceLoginState::PresentButUnsupported, expiry)
}

fn inspect(reference: &PrivateReference) -> Result<(SourceLoginState, Option<i64>), String> {
    if !matches!(
        reference.source,
        ModelSourceKind::Codex | ModelSourceKind::KimiCode
    ) {
        return Err("source_account_reference_not_supported".into());
    }
    if !std::path::Path::new(&reference.path).is_absolute() {
        return Err("source_path_must_be_absolute".into());
    }
    match std::fs::symlink_metadata(&reference.path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((SourceLoginState::LoginRequired, None))
        }
        Err(_) => return Ok((SourceLoginState::Unavailable, None)),
        Ok(_) => {}
    }
    let bytes = crate::model_connection_host::read_selected(&reference.path)?;
    let document: Value = match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => return Ok((SourceLoginState::Invalid, None)),
    };
    Ok(classify(
        reference.source,
        &document,
        chrono::Utc::now().timestamp_millis(),
    ))
}

fn status(
    account: ModelAccountSnapshot,
    state: SourceLoginState,
    expiry: Option<i64>,
) -> SourceAccountStatus {
    let action = match state {
        SourceLoginState::LoginRequired | SourceLoginState::Expired => Some(
            match account.account.source {
                ModelSourceKind::Codex => "login_in_codex_then_refresh",
                _ => "login_in_kimi_code_then_refresh",
            }
            .into(),
        ),
        SourceLoginState::PresentButUnsupported => {
            Some("use_explicit_api_key_connection_for_inference".into())
        }
        SourceLoginState::Invalid | SourceLoginState::Unavailable => {
            Some("check_explicit_source_credential_file".into())
        }
        SourceLoginState::Disconnected => None,
    };
    SourceAccountStatus {
        account,
        login_state: state,
        inference_supported: false,
        user_action: action,
        expires_at_ms: expiry,
    }
}

fn save(
    store: &NolaStore,
    account: ModelAccount,
    revision: Option<i64>,
) -> Result<ModelAccountSnapshot, String> {
    store.application()?.execute(ApplicationCommand::Model {
        command: ModelCommand::SaveAccount {
            input: account,
            expected_revision: revision,
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        },
    })
}

pub fn bind_source_account(
    store: &NolaStore,
    request: SourceAccountRequest,
) -> Result<SourceAccountStatus, String> {
    let reference = PrivateReference {
        source: request.source,
        path: request.path,
    };
    let (state, expiry) = inspect(&reference)?;
    let handle = format!("source-login-{}", uuid::Uuid::new_v4().simple());
    let account = ModelAccount {
        id: request.account_id,
        display_name: request.display_name,
        source: request.source,
        credential_ref: Some(handle.clone()),
        state: if state == SourceLoginState::Expired {
            ModelAccountState::Expired
        } else {
            ModelAccountState::Unavailable
        },
    };
    account.validate()?;
    // NolaStore is a lazy path handle. Open the canonical owner before its
    // sibling vault, and reject a stale account before writing a new slot.
    let current: Option<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::GetAccount {
                id: account.id.clone(),
            },
        })?;
    if current.as_ref().map(|snapshot| snapshot.revision) != request.expected_revision {
        return Err("source_account_changed_refresh".into());
    }
    // Only the selected path is stored privately, not the file or token bytes.
    let serialized = serde_json::to_string(&reference).map_err(|_| "source_reference_invalid")?;
    HostCredentialVault::open(store.path())?.write(
        &handle,
        REFERENCE_IDENTITY,
        0,
        Some(&serialized),
        || Ok(()),
    )?;
    Ok(status(
        save(store, account, request.expected_revision)?,
        state,
        expiry,
    ))
}

pub fn refresh_source_account(
    store: &NolaStore,
    account_id: &str,
    expected_revision: i64,
) -> Result<SourceAccountStatus, String> {
    let current: Option<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::GetAccount {
                id: account_id.into(),
            },
        })?;
    let mut current = current.ok_or("source_account_missing")?;
    if current.revision != expected_revision {
        return Err("source_account_revision_changed".into());
    }
    if current.account.state == ModelAccountState::Disconnected {
        return Ok(status(current, SourceLoginState::Disconnected, None));
    }
    let handle = current
        .account
        .credential_ref
        .as_deref()
        .filter(|v| v.starts_with("source-login-"))
        .ok_or("source_account_not_a_local_reference")?;
    let record = HostCredentialVault::open(store.path())?
        .record(handle)?
        .filter(|record| record.identity == REFERENCE_IDENTITY)
        .ok_or("source_reference_unavailable")?;
    let reference: PrivateReference = serde_json::from_str(
        record
            .secret
            .as_deref()
            .ok_or("source_reference_unavailable")?,
    )
    .map_err(|_| "source_reference_invalid")?;
    if reference.source != current.account.source {
        return Err("source_reference_identity_changed".into());
    }
    let (state, expiry) = inspect(&reference)?;
    current.account.state = if state == SourceLoginState::Expired {
        ModelAccountState::Expired
    } else {
        ModelAccountState::Unavailable
    };
    Ok(status(
        save(store, current.account, Some(expected_revision))?,
        state,
        expiry,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn login_presence_does_not_authorize_inference_and_expiry_is_explicit() {
        assert_eq!(
            classify(ModelSourceKind::Codex, &json!({}), 100).0,
            SourceLoginState::LoginRequired
        );
        assert_eq!(
            classify(
                ModelSourceKind::Codex,
                &json!({"tokens":{"access_token":"fixture-not-a-jwt"}}),
                100
            )
            .0,
            SourceLoginState::PresentButUnsupported
        );
        assert_eq!(
            classify(
                ModelSourceKind::KimiCode,
                &json!({"access_token":"fixture","expires_at":1}),
                2000
            )
            .0,
            SourceLoginState::Expired
        );
        assert_eq!(
            classify(
                ModelSourceKind::KimiCode,
                &json!({"access_token":"fixture","expires_at":10}),
                2000
            )
            .0,
            SourceLoginState::PresentButUnsupported
        );
    }
    #[test]
    fn local_reference_never_copies_source_credentials_or_changes_source_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture-credentials.json");
        let bytes = br#"{"access_token":"fixture-private-token","refresh_token":"fixture-private-refresh","expires_at":1}"#;
        std::fs::write(&path, bytes).unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let result = bind_source_account(
            &store,
            SourceAccountRequest {
                account_id: "source-fixture".into(),
                display_name: "Kimi source".into(),
                source: ModelSourceKind::KimiCode,
                path: path.to_str().unwrap().into(),
                expected_revision: None,
            },
        )
        .unwrap();
        assert_eq!(result.login_state, SourceLoginState::Expired);
        assert!(!result.inference_supported);
        let public = serde_json::to_string(&result).unwrap();
        assert!(!public.contains("fixture-private"));
        assert!(!public.contains(path.to_str().unwrap()));
        let record = HostCredentialVault::open(store.path())
            .unwrap()
            .record(result.account.account.credential_ref.as_ref().unwrap())
            .unwrap()
            .unwrap();
        assert!(!record.secret.unwrap().contains("fixture-private"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
