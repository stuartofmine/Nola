// SPDX-License-Identifier: Apache-2.0
//! Shared configuration paths; no argument parser or terminal dependencies.
use std::path::{Path, PathBuf};

pub fn expand_home(path: &Path) -> PathBuf {
    expand_home_from(path, crate::platform_paths::user_home_dir().as_deref())
}

pub(crate) fn expand_home_from(path: &Path, user_home: Option<&Path>) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return user_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = user_home {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

pub(crate) fn project_ancestor_directories(project_root: &Path) -> Vec<PathBuf> {
    let current = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let boundary = current
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| current.clone());
    let mut directories = current
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(&boundary))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    directories.reverse();
    directories
}
