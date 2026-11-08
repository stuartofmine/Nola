// SPDX-License-Identifier: Apache-2.0
//! Workspace access result data. Presentation paths never carry grant identity.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAccessGrant {
    #[serde(rename = "type")]
    result_type: WorkspaceAccessGrantType,
    root_resource_uri: String,
    default_working_directory_changed: bool,
    mutation_authorized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum WorkspaceAccessGrantType {
    #[serde(rename = "workspace_access_granted")]
    Granted,
}

impl WorkspaceAccessGrant {
    /// The Host supplies its canonical, OS-accessible directory after the
    /// concrete invocation is authorized. This constructor performs no IO or
    /// permission decision and cannot itself create execution authority.
    pub fn for_directory(root: &Path) -> Result<Self, String> {
        let uri = url::Url::from_directory_path(root)
            .map_err(|_| "Workspace access result needs an absolute directory".to_string())?;
        Ok(Self {
            result_type: WorkspaceAccessGrantType::Granted,
            root_resource_uri: uri.to_string(),
            default_working_directory_changed: false,
            mutation_authorized: false,
        })
    }

    /// Bind the returned identity to the already-authorized invocation. Even
    /// well-typed data cannot authorize an arbitrary root or broader parent.
    /// The caller owns OS canonicalization and authorization; this transport
    /// contract only checks identity and preserves exact filesystem bytes.
    pub fn directory_for_approved_root(&self, approved_root: &Path) -> Result<PathBuf, String> {
        if self.default_working_directory_changed || self.mutation_authorized {
            return Err(
                "Workspace access cannot change the default cwd or authorize mutation".into(),
            );
        }
        let uri = url::Url::parse(&self.root_resource_uri)
            .map_err(|_| "Invalid workspace directory resource URI".to_string())?;
        if uri.scheme() != "file" || uri.query().is_some() || uri.fragment().is_some() {
            return Err("Workspace access needs a local file resource URI".into());
        }
        let root = uri
            .to_file_path()
            .map_err(|_| "Workspace resource URI is not a local absolute path".to_string())?;
        if !approved_root.is_absolute() || root != approved_root {
            return Err("Workspace result does not match the approved directory".into());
        }
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_grant_round_trips_exact_approved_identity() {
        let root = std::env::temp_dir().join("approved workspace with spaces");
        let grant = WorkspaceAccessGrant::for_directory(&root).unwrap();
        let decoded: WorkspaceAccessGrant =
            serde_json::from_value(serde_json::to_value(&grant).unwrap()).unwrap();
        assert_eq!(decoded.directory_for_approved_root(&root).unwrap(), root);
        assert!(decoded
            .directory_for_approved_root(root.parent().unwrap())
            .is_err());
        assert!(WorkspaceAccessGrant::for_directory(Path::new("relative/workspace")).is_err());
    }

    #[test]
    fn workspace_grant_rejects_malformed_or_broader_authority() {
        let root = std::env::temp_dir().join("approved-workspace");
        let valid =
            serde_json::to_value(WorkspaceAccessGrant::for_directory(&root).unwrap()).unwrap();
        for field in [
            "type",
            "root_resource_uri",
            "default_working_directory_changed",
            "mutation_authorized",
        ] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<WorkspaceAccessGrant>(missing).is_err(),
                "{field}"
            );
        }
        for (field, invalid) in [
            ("type", json!("workspace_access_required")),
            ("root", json!(root.to_string_lossy())),
        ] {
            let mut malformed = valid.clone();
            malformed[field] = invalid;
            assert!(
                serde_json::from_value::<WorkspaceAccessGrant>(malformed).is_err(),
                "{field}"
            );
        }
        for (field, invalid) in [
            (
                "root_resource_uri",
                json!("https://example.test/workspace/"),
            ),
            (
                "root_resource_uri",
                json!("file://other-host/tmp/workspace/"),
            ),
            (
                "root_resource_uri",
                json!("file:///tmp/workspace/?authority=true"),
            ),
            (
                "root_resource_uri",
                json!("file:///tmp/workspace/#authority"),
            ),
            ("root_resource_uri", json!("not a URI")),
            (
                "root_resource_uri",
                json!(url::Url::from_directory_path(root.parent().unwrap())
                    .unwrap()
                    .as_str()),
            ),
            ("default_working_directory_changed", json!(true)),
            ("mutation_authorized", json!(true)),
        ] {
            let mut malformed = valid.clone();
            malformed[field] = invalid;
            let decoded: WorkspaceAccessGrant = serde_json::from_value(malformed).unwrap();
            assert!(
                decoded.directory_for_approved_root(&root).is_err(),
                "{field}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_grant_preserves_non_utf8_resource_identity() {
        use std::os::unix::ffi::OsStringExt;
        let exact =
            std::env::temp_dir().join(std::ffi::OsString::from_vec(b"workspace-\xff".to_vec()));
        let same_display = std::env::temp_dir().join("workspace-\u{fffd}");
        assert_eq!(exact.to_string_lossy(), same_display.to_string_lossy());
        let grant = WorkspaceAccessGrant::for_directory(&exact).unwrap();
        let data = serde_json::to_value(&grant).unwrap();
        let decoded: WorkspaceAccessGrant = serde_json::from_value(data).unwrap();
        assert_eq!(decoded.directory_for_approved_root(&exact).unwrap(), exact);
        assert!(decoded.directory_for_approved_root(&same_display).is_err());
    }
}
