// SPDX-License-Identifier: Apache-2.0
use super::{hub::Hub, FileSpec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn plain_metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(format!("Unexpected symbolic link: {}", path.display()))
        }
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
pub(super) fn plain_file_len(path: &Path) -> Result<Option<u64>, String> {
    match plain_metadata(path)? {
        Some(m) if m.is_file() => Ok(Some(m.len())),
        Some(_) => Err(format!("Expected a file at {}", path.display())),
        None => Ok(None),
    }
}
pub(super) fn plain_dir_exists(path: &Path) -> Result<bool, String> {
    match plain_metadata(path)? {
        Some(m) if m.is_dir() => Ok(true),
        Some(_) => Err(format!("Expected a directory at {}", path.display())),
        None => Ok(false),
    }
}
pub(super) fn directory(parent: &Path, component: &str) -> Result<PathBuf, String> {
    let path = parent.join(component);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    if !plain_dir_exists(&path)? {
        return Err("Model directory disappeared".into());
    }
    Ok(path)
}
pub(super) fn artifact_path(parent: &Path, relative: &str) -> Result<PathBuf, String> {
    if !super::selection::safe_path(relative) {
        return Err("Invalid artifact path".into());
    }
    let mut dir = parent.to_owned();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok(dir.join(part));
        }
        dir = directory(&dir, part)?;
    }
    Err("Missing filename".into())
}
pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let size = plain_file_len(path)?.ok_or_else(|| format!("{} not found", path.display()))?;
    if size > 64 * 1024 * 1024 {
        return Err(format!("JSON document is too large: {}", path.display()));
    }
    serde_json::from_reader(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid {}: {e}", path.display()))
}
pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent directory")?)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(tmp.as_file_mut(), value).map_err(|e| e.to_string())?;
    tmp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub(crate) struct Lock {
    dir: PathBuf,
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.dir.join("owner.json"));
        let _ = fs::remove_dir(&self.dir);
    }
}
#[cfg(unix)]
fn live(pid: u32) -> bool {
    pid>0 && i32::try_from(pid).is_ok_and(|n|unsafe{libc::kill(n,0)==0}||io::Error::last_os_error().raw_os_error()==Some(libc::EPERM))
}
#[cfg(windows)]
fn live(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
        }
        let mut code = 0;
        let active = GetExitCodeProcess(handle, &mut code) == 0 || code == 259;
        CloseHandle(handle);
        active
    }
}
fn lock(dir: PathBuf) -> Result<Lock, String> {
    let parent = dir.parent().ok_or("Missing lock directory")?;
    // Serialize stale-owner recovery across native installers. The directory itself
    // is also respected by the existing Python installer.
    let guard_path = parent.join(format!(
        "{}.guard",
        dir.file_name().unwrap().to_string_lossy()
    ));
    plain_file_len(&guard_path)?;
    let guard = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&guard_path)
        .map_err(|e| e.to_string())?;
    guard.lock().map_err(|e| e.to_string())?;
    if plain_dir_exists(&dir)? {
        let owner = read_json(&dir.join("owner.json")).ok();
        let pid = owner
            .as_ref()
            .and_then(|v| v["pid"].as_u64())
            .and_then(|n| u32::try_from(n).ok());
        let old = fs::metadata(&dir)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(300));
        if pid.map_or(!old, live) {
            return Err("Another model operation is in progress. Retry when it finishes.".into());
        }
        if dir.join("owner.json").exists() {
            fs::remove_file(dir.join("owner.json")).map_err(|e| e.to_string())?;
        }
        fs::remove_dir(&dir).map_err(|e| e.to_string())?;
    }
    // Publish an already-populated lock so another installer always sees a PID.
    let pending = tempfile::Builder::new()
        .prefix(".lock-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    write_json(
        &pending.path().join("owner.json"),
        &json!({"schemaVersion":1,"pid":std::process::id(),"taskID":uuid::Uuid::new_v4().to_string(),"startedEpoch":epoch,"heartbeatEpoch":epoch}),
    )?;
    fs::rename(pending.path(), &dir).map_err(|e| format!("Cannot acquire model lock: {e}"))?;
    Ok(Lock { dir })
}
pub(crate) fn model_lock(model_dir: &Path) -> Result<Lock, String> {
    lock(model_dir.join(".install.lock"))
}

// Earlier released CLI writers serialized only InstalledModelCatalog.models.
fn legacy_catalog(catalog: &Value) -> bool {
    catalog.as_object().is_some_and(|obj| obj.len() == 1)
        && catalog["models"].as_array().is_some_and(|models| {
            models.iter().all(|m| {
                [
                    "id",
                    "name",
                    "repository",
                    "runtime",
                    "format",
                    "artifactPath",
                    "state",
                ]
                .iter()
                .all(|key| m[*key].is_string())
                    && m["id"].as_str().is_some_and(|id| !id.is_empty())
                    && m["contextWindowTokens"].as_u64().is_some()
                    && m["artifactBytes"].as_u64().is_some()
                    && m["enabled"].is_boolean()
            })
        })
}

pub(crate) fn read_catalog(root: &Path) -> Result<Value, String> {
    let path = root.join("catalog.json");
    let mut catalog = if plain_file_len(&path)?.is_some() {
        read_json(&path)?
    } else {
        json!({"schemaVersion":1,"models":[]})
    };
    if catalog.get("schemaVersion").is_none() && legacy_catalog(&catalog) {
        catalog["schemaVersion"] = json!(1);
    }
    if !matches!(catalog["schemaVersion"].as_u64(), Some(1 | 2)) {
        return Err("Unsupported local model catalog version".into());
    }
    if !catalog["models"].is_array() {
        return Err("Invalid local model catalog".into());
    }
    Ok(catalog)
}

pub(crate) fn update_catalog(
    root: &Path,
    edit: impl FnOnce(&mut Vec<Value>) -> Result<(), String>,
) -> Result<(), String> {
    let _guard = lock(root.join(".catalog.lock"))?;
    let path = root.join("catalog.json");
    let mut catalog = read_catalog(root)?;
    let items = catalog["models"]
        .as_array_mut()
        .ok_or("Invalid local model catalog")?;
    edit(items)?;
    write_json(&path, &catalog)
}

pub(super) fn download(
    hub: &Hub,
    repo: &str,
    file: &FileSpec,
    path: &Path,
    progress: impl FnMut(u64) -> bool,
) -> Result<(), String> {
    download_with(file, path, progress, |offset| {
        hub.artifact(repo, file, offset)
    })
}
fn download_with(
    file: &FileSpec,
    path: &Path,
    mut progress: impl FnMut(u64) -> bool,
    fetch: impl FnOnce(u64) -> Result<reqwest::blocking::Response, String>,
) -> Result<(), String> {
    if plain_file_len(path)? == Some(file.bytes) {
        return Ok(());
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let mut offset = plain_file_len(&partial)?.unwrap_or(0);
    if offset > file.bytes {
        fs::remove_file(&partial).map_err(|e| e.to_string())?;
        offset = 0;
    }
    if !progress(offset) {
        return Err("Model download cancelled".into());
    }
    if file.bytes == 0 && plain_file_len(&partial)?.is_none() {
        File::create_new(&partial).map_err(|e| e.to_string())?;
    }
    let mut digest = Sha256::new();
    if offset < file.bytes {
        let mut response = fetch(offset)?;
        let range = format!("bytes {}-{}/{}", offset, file.bytes - 1, file.bytes);
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            if response
                .headers()
                .get("content-range")
                .and_then(|s| s.to_str().ok())
                != Some(range.as_str())
            {
                return Err(format!("Invalid range for {}", file.path));
            }
        } else {
            offset = 0;
        }
        if response
            .content_length()
            .is_some_and(|n| n != file.bytes - offset)
        {
            return Err(format!("Unexpected byte count for {}", file.path));
        }
        if offset > 0 && file.sha256.is_some() {
            io::copy(
                &mut File::open(&partial).map_err(|e| e.to_string())?,
                &mut digest,
            )
            .map_err(|e| e.to_string())?;
        }
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&partial)
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut n = offset;
        loop {
            let count = response
                .read(&mut buffer)
                .map_err(|e| format!("Downloading {}: {e}", file.path))?;
            if count == 0 {
                break;
            }
            n += count as u64;
            if n > file.bytes {
                return Err("Model download exceeded declared size".into());
            }
            out.write_all(&buffer[..count])
                .map_err(|e| format!("Writing {}: {e}", file.path))?;
            if file.sha256.is_some() {
                digest.update(&buffer[..count]);
            }
            if !progress(n) {
                return Err("Model download cancelled".into());
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
        if n != file.bytes {
            return Err(format!("Incomplete download for {}", file.path));
        }
    } else if file.sha256.is_some() {
        let mut data = File::open(&partial).map_err(|e| e.to_string())?;
        io::copy(&mut data, &mut digest).map_err(|e| e.to_string())?;
    }
    if let Some(expected) = &file.sha256 {
        if hex::encode(digest.finalize()) != *expected {
            fs::remove_file(&partial).map_err(|e| e.to_string())?;
            return Err(format!("Repository checksum mismatch for {}", file.path));
        }
    }
    fs::rename(&partial, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unknown_data_and_rejects_unreadable_catalogs() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        for bad in [
            b"corrupt".as_slice(),
            br#"{"schemaVersion":80,"models":[]}"#,
        ] {
            fs::write(&path, bad).unwrap();
            assert!(update_catalog(root.path(), |_| Ok(())).is_err());
            assert_eq!(fs::read(&path).unwrap(), bad);
        }
        write_json(
            &path,
            &json!({"schemaVersion":2,"opaque":true,"models":[{"legacy":42}]}),
        )
        .unwrap();
        update_catalog(root.path(), |m| {
            m.push(json!({"id":"a"}));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            read_json(&path).unwrap(),
            json!({"schemaVersion":2,"opaque":true,"models":[{"legacy":42},{"id":"a"}]})
        );
        let held = lock(root.path().join(".catalog.lock")).unwrap();
        assert!(update_catalog(root.path(), |_| Ok(())).is_err());
        drop(held);
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_artifact_directories() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("nested")).unwrap();
        assert!(artifact_path(root.path(), "nested/model.gguf").is_err());
    }
}

#[cfg(unix)]
pub(super) fn free_space(dir: &Path) -> io::Result<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(dir.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok((info.f_bavail as u64).saturating_mul(info.f_frsize as u64))
}
#[cfg(windows)]
pub(super) fn free_space(dir: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}
pub(super) fn space_required(total: u64, completed: u64) -> u64 {
    total
        .saturating_sub(completed)
        .saturating_add((total / 20).max(512 * 1024 * 1024))
}
pub(super) fn completed_bytes(dir: &Path, files: &[FileSpec]) -> Result<u64, String> {
    let mut completed = 0;
    for file in files {
        let mut path = dir.to_path_buf();
        let mut components = file.path.split('/').peekable();
        while let Some(c) = components.next() {
            path.push(c);
            if components.peek().is_some() {
                plain_dir_exists(&path)?;
            }
        }
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let bytes = if plain_file_len(&path)? == Some(file.bytes) {
            file.bytes
        } else {
            plain_file_len(Path::new(&part))?
                .filter(|n| *n <= file.bytes)
                .unwrap_or(0)
        };
        completed += bytes;
    }
    Ok(completed)
}
pub(super) fn check_space(dir: &Path, files: &[FileSpec], total: u64) -> Result<(), String> {
    let completed = completed_bytes(dir, files)?;
    let required = space_required(total, completed);
    let free = free_space(dir).map_err(|e| format!("Cannot check available disk space: {e}"))?;
    if free < required {
        return Err(format!(
            "Insufficient disk space for this model: {required} bytes required, {free} available"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod acquisition_storage_tests {
    use super::*;
    #[test]
    fn legacy_catalog_preserves_existing_models_and_rejects_malformed_variants() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        let previous = json!({"id":"legacy","name":"Legacy","repository":"test/model","runtime":"llama.cpp","format":"gguf","artifactPath":"models/a.gguf","artifactBytes":20,"state":"configured","enabled":true,"contextWindowTokens":4096,"extraField":true});
        write_json(&path, &json!({"models":[previous.clone()]})).unwrap();
        update_catalog(root.path(), |models| {
            models.push(json!({"id":"new"}));
            Ok(())
        })
        .unwrap();
        let catalog = read_json(&path).unwrap();
        assert_eq!(catalog["models"][0], previous);
        assert_eq!(catalog["schemaVersion"], 1);
        for invalid in [
            json!({"schemaVersion":null,"models":[]}),
            json!({"models":[{}]}),
            json!({"unknown":true,"models":[]}),
        ] {
            write_json(&path, &invalid).unwrap();
            assert!(update_catalog(root.path(), |_| Ok(())).is_err());
            assert_eq!(read_json(&path).unwrap(), invalid);
        }
    }
    #[test]
    fn resume_requires_the_original_range_and_published_checksum() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("weights");
        let part = root.path().join("weights.part");
        let bytes = b"hello again";
        let file = FileSpec {
            path: "weights".into(),
            bytes: bytes.len() as u64,
            revision: "a".repeat(40),
            sha256: Some(hex::encode(Sha256::digest(bytes))),
        };
        fs::write(&part, b"hello").unwrap();
        for (range, body, good) in [
            ("bytes 3-8/11", " again", false),
            ("bytes 5-10/11", " again", true),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let address = server.server_addr().to_string();
            let handle = std::thread::spawn(move || {
                let req = server.recv().unwrap();
                assert!(req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Range") && h.value.as_str() == "bytes=5-"));
                let response = tiny_http::Response::from_string(body)
                    .with_status_code(206)
                    .with_header(tiny_http::Header::from_bytes("Content-Range", range).unwrap());
                req.respond(response).unwrap();
            });
            let result = download_with(
                &file,
                &path,
                |_| true,
                |offset| {
                    reqwest::blocking::Client::builder()
                        .no_proxy()
                        .build()
                        .unwrap()
                        .get(format!("http://{address}/weights"))
                        .header("Range", format!("bytes={offset}-"))
                        .send()
                        .map_err(|e| e.to_string())
                },
            );
            handle.join().unwrap();
            assert_eq!(result.is_ok(), good);
            if !good {
                assert_eq!(fs::read(&part).unwrap(), b"hello");
                assert!(!path.exists());
            }
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
