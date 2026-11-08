// SPDX-License-Identifier: Apache-2.0
//! Host-configured language-service launch data. Configuration is not an
//! authorization grant; Core must admit the concrete process and project.

use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageServiceConfiguration {
    pub id: String,
    /// Absolute executable, not a shell fragment or PATH-dependent alias.
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub project_root: PathBuf,
    pub language_ids: Vec<String>,
    /// Explicit environment only. The launcher must not inherit credentials.
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: u64,
}

impl std::fmt::Debug for LanguageServiceConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("LanguageServiceConfiguration")
            .field("id", &self.id)
            .field("language_ids", &self.language_ids)
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

impl LanguageServiceConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        let valid_id = |value: &str| !value.is_empty() && value.len() <= 128
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid_id(&self.id) { return Err("invalid language service ID".into()); }
        if !self.executable.is_absolute() || !self.project_root.is_absolute() {
            return Err("language service executable and project root must be absolute".into());
        }
        for path in [&self.executable, &self.project_root] {
            let text = path.to_str().ok_or("language service paths must be UTF-8")?;
            if text.len() > 4096 || text.chars().any(char::is_control) {
                return Err("invalid language service path".into());
            }
        }
        if self.language_ids.is_empty() || self.language_ids.len() > 32
            || self.language_ids.iter().any(|id| !valid_id(id))
            || self.language_ids.iter().collect::<BTreeSet<_>>().len() != self.language_ids.len() {
            return Err("language service needs unique language IDs".into());
        }
        if !(100..=120_000).contains(&self.timeout_ms) {
            return Err("language service timeout must be 100..120000 milliseconds".into());
        }
        if self.arguments.len() > 128 || self.arguments.iter().any(|arg| arg.contains('\0'))
            || self.arguments.iter().map(String::len).sum::<usize>() > 64 * 1024 {
            return Err("invalid or oversized language service arguments".into());
        }
        if self.environment.len() > 128 || self.environment.iter().any(|(key, value)| {
            key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
        }) || self.environment.iter().map(|(key,value)| key.len() + value.len()).sum::<usize>() > 64 * 1024 {
            return Err("invalid or oversized language service environment".into());
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

pub fn validate_language_services(services: &[LanguageServiceConfiguration]) -> Result<(), String> {
    if services.len() > 32 { return Err("at most 32 language services may be configured".into()); }
    let mut names = BTreeSet::new();
    for service in services {
        service.validate()?;
        if !names.insert(&service.id) { return Err("duplicate language service ID".into()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration() -> LanguageServiceConfiguration {
        LanguageServiceConfiguration {
            id:"rust".into(), executable:PathBuf::from("/tools/rust-analyzer"),
            arguments:vec![], project_root:PathBuf::from("/workspace"),
            language_ids:vec!["rust".into()], environment:BTreeMap::new(), timeout_ms:10_000,
        }
    }

    #[test]
    fn launch_configuration_is_bounded_and_fingerprinted() {
        let base = configuration();
        validate_language_services(&[base.clone()]).unwrap();
        assert!(validate_language_services(&[base.clone(),base.clone()]).is_err());
        let mut changed = base.clone();
        changed.environment.insert("MODE".into(),"different".into());
        assert_ne!(base.fingerprint().unwrap(), changed.fingerprint().unwrap());
        changed.executable = PathBuf::from("rust-analyzer");
        assert!(changed.validate().is_err());
        changed = base.clone();
        changed.arguments.push("bad\0argument".into());
        assert!(changed.validate().is_err());
        changed = base;
        changed.timeout_ms = 0;
        assert!(changed.validate().is_err());
    }
}
