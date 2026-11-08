// SPDX-License-Identifier: Apache-2.0
//! Bounded plugin package inspection. No installer scripts or plugin code run.
//! The captured bytes, not the mutable source directory, are materialized.

use nola_domain::plugin::{PluginExecutable, PluginManifest, PluginMcpTransport};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::{Read, Write}, path::{Path, PathBuf}};

mod runtime;
pub use runtime::{prepare_runtime, PluginRuntime, RuntimeOwner, RuntimeSkill};

const MAX_FILES: usize = 4096;
const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

struct CapturedFile { bytes: Vec<u8>, executable: bool }

pub struct InspectedPackage {
    manifest: PluginManifest,
    digest: String,
    files: BTreeMap<String, CapturedFile>,
}

impl InspectedPackage {
    pub fn manifest(&self) -> &PluginManifest { &self.manifest }
    pub fn digest(&self) -> &str { &self.digest }
    pub fn byte_count(&self) -> usize { self.files.values().map(|f| f.bytes.len()).sum() }

    /// Publish an immutable content-addressed snapshot before Application
    /// records installation. Failed admission may leave an unreferenced package,
    /// never a registry row that points to a half-written directory.
    pub fn publish(&self, package_store: &Path) -> Result<PathBuf, String> {
        let stage = self.stage(package_store)?;
        let destination = package_store.join(&self.digest);
        if destination.exists() {
            verify_installed(package_store, &self.digest)?;
            return Ok(destination);
        }
        // Flush child directories as well as files before publishing the root.
        #[cfg(unix)] {
            let mut directories = std::collections::BTreeSet::new();
            for relative in self.files.keys() {
                let mut parent = Path::new(relative).parent();
                while let Some(path) = parent {
                    directories.insert(path.to_path_buf());
                    parent = path.parent();
                }
            }
            for directory in directories.iter().rev() {
                fs::File::open(stage.path().join(directory)).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
            }
            fs::File::open(stage.path()).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(stage.path(), &destination) {
            // A concurrent publication of the same snapshot is harmless only
            // after verifying its actual bytes; never overwrite a different one.
            if verify_installed(package_store, &self.digest).is_err() { return Err(error.to_string()); }
        }
        #[cfg(unix)]
        fs::File::open(package_store).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        Ok(destination)
    }

    /// Writes only into a new private staging directory. Application must admit
    /// the resulting package before activation; dropping staging removes it.
    pub fn stage(&self, package_store: &Path) -> Result<tempfile::TempDir, String> {
        fs::create_dir_all(package_store).map_err(|e| e.to_string())?;
        let stage = tempfile::Builder::new().prefix(".plugin-stage-")
            .tempdir_in(package_store).map_err(|e| e.to_string())?;
        for (relative, captured) in &self.files {
            let destination = stage.path().join(relative);
            fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
            let mut file = fs::OpenOptions::new().write(true).create_new(true)
                .open(&destination).map_err(|e| e.to_string())?;
            file.write_all(&captured.bytes).map_err(|e| e.to_string())?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(if captured.executable { 0o700 } else { 0o600 }))
                    .map_err(|e| e.to_string())?;
            }
            file.sync_all().map_err(|e| e.to_string())?;
        }
        Ok(stage)
    }
}

pub fn verify_installed(package_store: &Path, digest: &str) -> Result<InspectedPackage, String> {
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("invalid package digest".into());
    }
    let directory = package_store.join(digest);
    if fs::symlink_metadata(&directory).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("installed package root cannot be a symlink".into());
    }
    let package = inspect(&directory)?;
    if package.digest != digest { return Err("installed plugin content digest mismatch".into()); }
    Ok(package)
}

/// Every component is opened relative to an already-open directory descriptor;
/// O_NOFOLLOW prevents symlink replacement between enumeration and opening.
#[cfg(unix)]
fn open_beneath(root: &fs::File, relative: &Path, directory: bool) -> Result<fs::File, String> {
    use std::{ffi::CString, os::{fd::{AsRawFd, FromRawFd}, unix::ffi::OsStrExt}};
    let components: Vec<_> = relative.components().collect();
    let mut parent = root.try_clone().map_err(|e| e.to_string())?;
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else { return Err("invalid package component".into()); };
        let name = CString::new(name.as_bytes()).map_err(|_| "invalid package filename")?;
        let is_dir = index + 1 < components.len() || directory;
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
            | if is_dir { libc::O_DIRECTORY } else { 0 };
        // SAFETY: parent and CString remain alive, no creation mode is needed.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 { return Err(format!("unsafe/unreadable package entry: {}", std::io::Error::last_os_error())); }
        // SAFETY: openat returned a fresh owned descriptor.
        parent = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(parent)
}

#[cfg(not(unix))]
fn open_beneath(_: &fs::File, _: &Path, _: bool) -> Result<fs::File, String> {
    Err("secure plugin package inspection is not implemented for this host".into())
}

pub fn inspect(source: &Path) -> Result<InspectedPackage, String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC);
    }
    let root = options.open(&source).map_err(|e| e.to_string())?;
    if !root.metadata().map_err(|e| e.to_string())?.is_dir() { return Err("plugin root must be a directory".into()); }
    let mut pending = vec![PathBuf::new()];
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    let mut entries = 0usize;
    while let Some(directory) = pending.pop() {
        // Enumeration only supplies names. All content is read using the root
        // descriptor, even if an ancestor is renamed during enumeration.
        for entry in fs::read_dir(source.join(&directory)).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            entries += 1;
            if entries > MAX_FILES { return Err("plugin package entry budget exceeded".into()); }
            let relative = directory.join(entry.file_name());
            let name = relative.to_str().ok_or("plugin paths must be UTF-8")?.to_owned();
            nola_domain::plugin::validate_package_path(&format!("./{name}"))?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() { return Err("plugin package symlinks are not permitted".into()); }
            if kind.is_dir() {
                open_beneath(&root, &relative, true)?;
                pending.push(relative);
                continue;
            }
            let file = open_beneath(&root, &relative, false)?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() { return Err("plugin entry must be a regular file".into()); }
            let mut bytes = Vec::new();
            file.take((MAX_PACKAGE_BYTES - total + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            total += bytes.len();
            if total > MAX_PACKAGE_BYTES { return Err("plugin package byte budget exceeded".into()); }
            #[cfg(unix)]
            let executable = { use std::os::unix::fs::PermissionsExt; metadata.permissions().mode() & 0o111 != 0 };
            #[cfg(not(unix))]
            let executable = false;
            files.insert(name, CapturedFile { bytes, executable });
        }
    }
    let manifest = PluginManifest::decode(&files.get("nola-plugin.json").ok_or("missing nola-plugin.json")?.bytes)?;
    for skill in &manifest.skills {
        let path = format!("{}/SKILL.md", skill.path.trim_start_matches("./"));
        if !files.contains_key(&path) { return Err(format!("missing Skill instructions: {}", skill.id)); }
    }
    for server in &manifest.mcp_servers {
        if let PluginMcpTransport::Stdio { executable: PluginExecutable::Package { path }, .. } = &server.transport {
            let file = files.get(path.trim_start_matches("./")).ok_or("missing packaged executable")?;
            if !file.executable { return Err("packaged executable lacks execution permission".into()); }
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"nola-plugin-package-v1\0");
    for (name, file) in &files {
        hash.update((name.len() as u64).to_le_bytes()); hash.update(name);
        hash.update([u8::from(file.executable)]);
        hash.update((file.bytes.len() as u64).to_le_bytes()); hash.update(&file.bytes);
    }
    Ok(InspectedPackage { manifest, digest: format!("{:x}", hash.finalize()), files })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("skills/test")).unwrap();
        fs::write(dir.path().join("skills/test/SKILL.md"), "# Fixture\nDo not execute anything.").unwrap();
        fs::write(dir.path().join("nola-plugin.json"), r#"{"schema_version":1,"id":"fixture","version":"1","display_name":"Fixture","description":"Test","skills":[{"id":"test","path":"./skills/test"}]}"#).unwrap();
        dir
    }
    #[test]
    fn stages_captured_bytes_not_later_source_edits() {
        let source = fixture(); let captured = inspect(source.path()).unwrap();
        fs::write(source.path().join("skills/test/SKILL.md"), "replaced").unwrap();
        let store = tempfile::tempdir().unwrap(); let staged = captured.stage(store.path()).unwrap();
        assert_eq!(inspect(staged.path()).unwrap().digest(), captured.digest());
        assert_ne!(inspect(source.path()).unwrap().digest(), captured.digest());
        assert!(fs::read_to_string(staged.path().join("skills/test/SKILL.md")).unwrap().starts_with("# Fixture"));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_missing_resources() {
        let source = fixture();
        std::os::unix::fs::symlink("/etc/passwd", source.path().join("leak")).unwrap();
        assert!(inspect(source.path()).is_err());
        fs::remove_file(source.path().join("leak")).unwrap();
        fs::remove_file(source.path().join("skills/test/SKILL.md")).unwrap();
        assert!(inspect(source.path()).is_err());
    }
    #[test]
    fn deterministic_digest_includes_unlisted_resources() {
        let source = fixture(); let before = inspect(source.path()).unwrap().digest;
        assert_eq!(inspect(source.path()).unwrap().digest(), before);
        fs::write(source.path().join("helper.txt"), "helper").unwrap();
        assert_ne!(inspect(source.path()).unwrap().digest(), before);
    }
    #[test]
    fn publication_is_idempotent_and_tampering_is_not_repaired_silently() {
        let source = fixture(); let package = inspect(source.path()).unwrap();
        let store = tempfile::tempdir().unwrap();
        let destination = package.publish(store.path()).unwrap();
        assert_eq!(package.publish(store.path()).unwrap(), destination);
        assert_eq!(verify_installed(store.path(), package.digest()).unwrap().manifest(), package.manifest());
        fs::write(destination.join("skills/test/SKILL.md"), "tampered").unwrap();
        assert!(verify_installed(store.path(), package.digest()).is_err());
        assert!(package.publish(store.path()).is_err());
        assert_eq!(fs::read_to_string(destination.join("skills/test/SKILL.md")).unwrap(), "tampered");
    }
    #[cfg(unix)]
    #[test]
    fn executable_inspection_and_staging_never_start_the_server() {
        use std::os::unix::fs::PermissionsExt;
        let source = fixture();
        let server = source.path().join("server.sh");
        let marker = source.path().join("EXECUTED").to_string_lossy().replace('\'', "'\\''");
        fs::write(&server, format!("#!/bin/sh\ntouch '{marker}'\n")).unwrap();
        fs::set_permissions(&server, fs::Permissions::from_mode(0o700)).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(source.path().join("nola-plugin.json")).unwrap()).unwrap();
        manifest["mcp_servers"] = serde_json::json!([{"id":"server", "transport":{
            "type":"stdio", "executable":{"type":"package","path":"./server.sh"}}}]);
        fs::write(source.path().join("nola-plugin.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let package = inspect(source.path()).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let staged = package.stage(destination.path()).unwrap();
        assert!(!source.path().join("EXECUTED").exists());
        assert!(!staged.path().join("EXECUTED").exists());
        assert_eq!(fs::metadata(staged.path().join("server.sh")).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
