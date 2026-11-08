// SPDX-License-Identifier: Apache-2.0
//! Process-local observations published only alongside a committed generation.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

pub(super) fn stamps(paths: &[PathBuf]) -> HashMap<PathBuf, FileStamp> {
    paths
        .iter()
        .filter_map(|path| stamp(path).map(|stamp| (path.clone(), stamp)))
        .collect()
}

#[cfg(unix)]
fn stamp(path: &Path) -> Option<FileStamp> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(FileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

// Without a trustworthy change identity, continue hashing contents. Equal
// length and mtime alone cannot prove unchanged bytes (e.g. restored mtimes).
#[cfg(not(unix))]
fn stamp(_path: &Path) -> Option<FileStamp> {
    None
}

#[derive(Clone)]
pub(super) struct WorkspaceSnapshot {
    pub report: IndexReport,
    pub paths: Vec<PathBuf>,
    pub stamps: HashMap<PathBuf, FileStamp>,
}
