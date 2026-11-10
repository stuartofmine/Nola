// SPDX-License-Identifier: Apache-2.0
//! Explicit product management, never model-prompt keyword routing.
use crate::{application_port::CliApplicationPort, config::PluginCommandMode};
use nola_application::{ApplicationCommand, PluginCommand};
use nola_domain::plugin::PluginInstallation;
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

pub(crate) fn package_store() -> Result<PathBuf, String> {
    Ok(crate::platform_paths::application_database_path()?.with_extension("plugins").join("packages"))
}
fn get(app: &mut CliApplicationPort, id: &str) -> Result<Option<PluginInstallation>, String> {
    app.execute(ApplicationCommand::Plugin { command: PluginCommand::Get { id: id.into() } })
}
pub(crate) fn run(command: &PluginCommandMode) -> Result<Value, String> {
    if let PluginCommandMode::Inspect(directory) = command {
        let package = nola_plugin_host::inspect(Path::new(directory))?;
        return Ok(json!({"manifest":package.manifest(), "package_digest":package.digest(),
            "package_bytes":package.byte_count(), "installed":false, "executed":false}));
    }
    let mut app = CliApplicationPort::persistent_default()?;
    if let PluginCommandMode::CredentialSet { id, reference } | PluginCommandMode::CredentialClear { id, reference } = command {
        use std::io::{IsTerminal, Read};
        let record = get(&mut app, id)?.filter(|r| r.removed_at_ms.is_none()).ok_or("plugin not installed")?;
        if !declares_secret(&record, reference) { return Err("credential reference is not declared by this plugin".into()); }
        // Rebinding during an active instance would leave a cached secret in
        // transport. Require explicit disable, bind, enable instead.
        if record.enabled { return Err("disable the plugin before changing its credentials".into()); }
        let secret = if matches!(command, PluginCommandMode::CredentialSet { .. }) {
            if std::io::stdin().is_terminal() { return Err("provide the credential through stdin, not command arguments".into()); }
            let mut bytes = Vec::new();
            std::io::stdin().take(16_385).read_to_end(&mut bytes).map_err(|_| "cannot read credential")?;
            if bytes.len() > 16_384 { return Err("credential exceeds byte limit".into()); }
            let value = String::from_utf8(bytes).map_err(|_| "credential must be UTF-8")?;
            let value = value.trim_end_matches(['\r', '\n']).to_string();
            if value.is_empty() || value.contains('\0') { return Err("invalid credential".into()); }
            Some(value)
        } else { None };
        let mut vault = crate::host_credentials::HostCredentialVault::open(&crate::platform_paths::application_database_path()?)?;
        let baseline = vault.plugin_record(id, reference)?.map_or(0, |r| r.revision);
        let revision = vault.write_plugin(id, reference, &record.package_digest, baseline, secret.as_deref(), || {
            let current = get(&mut CliApplicationPort::persistent_default()?, id)?;
            if current.as_ref().is_some_and(|r| !r.enabled && r.removed_at_ms.is_none()
                && r.revision == record.revision && r.package_digest == record.package_digest) { Ok(()) }
            else { Err("plugin changed while binding credential".into()) }
        })?;
        return Ok(json!({"plugin_id":id,"reference":reference,"credential_revision":revision,"has_credential":secret.is_some()}));
    }
    if let PluginCommandMode::List { after } = command {
        let installations: Vec<PluginInstallation> = app.execute(ApplicationCommand::Plugin {
            command: PluginCommand::List { after: after.clone(), limit: 128 },
        })?;
        // A full page may be the last one; an extra empty page is harmless.
        // Never silently truncate the inventory or load every package at once.
        let next_after = (installations.len() == 128)
            .then(|| installations.last().unwrap().manifest.id.clone());
        return Ok(json!({"installations": installations, "next_after": next_after}));
    }
    let now_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?.as_millis()).map_err(|e| e.to_string())?;
    let request = match command {
        PluginCommandMode::Install(directory) => {
            let package = nola_plugin_host::inspect(Path::new(directory))?;
            let previous = get(&mut app, &package.manifest().id)?;
            if previous.as_ref().is_some_and(|record| record.enabled) {
                return Err("disable the installed plugin before replacing its package".into());
            }
            package.publish(&package_store()?)?;
            PluginCommand::Install { manifest: package.manifest().clone(), package_digest: package.digest().into(),
                expected_revision: previous.map(|record| record.revision), now_ms }
        }
        PluginCommandMode::Enable(id) | PluginCommandMode::Disable(id) | PluginCommandMode::Remove(id) => {
            let record = get(&mut app, id)?.filter(|r| r.removed_at_ms.is_none()).ok_or("plugin not installed")?;
            if matches!(command, PluginCommandMode::Enable(_)) {
                let package = nola_plugin_host::verify_installed(&package_store()?, &record.package_digest)?;
                if package.manifest() != &record.manifest { return Err("plugin manifest does not match installation".into()); }
                let mut candidate = record.clone();
                candidate.enabled = true;
                // Validate dependencies and credential bindings before storing
                // enabled intent. Preparation never launches plugin code.
                drop(crate::plugin_runtime::prepare_installed(&candidate)?);
            }
            if matches!(command, PluginCommandMode::Remove(_)) {
                PluginCommand::Remove { id: id.clone(), expected_revision: record.revision, now_ms }
            } else {
                PluginCommand::SetEnabled { id: id.clone(), expected_revision: record.revision,
                    enabled: matches!(command, PluginCommandMode::Enable(_)), now_ms }
            }
        }
        PluginCommandMode::Inspect(_) | PluginCommandMode::List { .. } => unreachable!(),
        PluginCommandMode::CredentialSet { .. } | PluginCommandMode::CredentialClear { .. } => unreachable!(),
    };
    app.execute(ApplicationCommand::Plugin { command: request })
}

fn declares_secret(record: &PluginInstallation, reference: &str) -> bool {
    use nola_domain::plugin::PluginMcpTransport;
    record.manifest.mcp_servers.iter().any(|server| match &server.transport {
        PluginMcpTransport::Stdio { secret_env, .. } => secret_env.values().any(|value| value == reference),
        PluginMcpTransport::StreamableHttp { bearer_secret_ref, .. } => bearer_secret_ref.as_deref() == Some(reference),
    })
}
