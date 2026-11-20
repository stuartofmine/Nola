// SPDX-License-Identifier: Apache-2.0
//! Discover local HTML contributions only in packages present in the runtime owner's inventory.
use super::{available, installed, registry_source, Harness};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_HTML: u64 = 2 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct Declaration {
    id: String,
    title: String,
    entry: String,
}

#[derive(Serialize)]
struct Page {
    id: String,
    title: String,
    package: String,
    #[serde(skip)]
    root: PathBuf,
    #[serde(skip)]
    entry: PathBuf,
}

fn package_name(name: &str) -> bool {
    registry_source(name) && !name.trim_start_matches('@').contains('@')
}

fn git_checkout(profile: &Path, source: &str) -> Option<PathBuf> {
    let input = source
        .strip_prefix("git:")
        .filter(|s| !s.starts_with("//"))
        .unwrap_or(source);
    let spec = if let Some(scp) = input.strip_prefix("git@") {
        let (host, path) = scp.split_once(':')?;
        format!("ssh://{host}/{path}")
    } else if input.contains("://") {
        input.to_owned()
    } else if let Some(path) = input.strip_prefix("github:") {
        format!("https://github.com/{path}")
    } else if input
        .split_once('/')
        .is_some_and(|(owner, _)| !owner.contains(['.', ':']))
    {
        format!("https://github.com/{input}")
    } else {
        format!("https://{input}")
    };
    let url = reqwest::Url::parse(&spec).ok()?;
    let path = url
        .path()
        .trim_matches('/')
        .split('@')
        .next()?
        .trim_end_matches(".git");
    let host = url.host_str()?;
    if !relative(host) || matches!(host, "." | "..") || !relative(path) {
        return None;
    }
    Some(profile.join("git").join(host).join(path))
}

fn package_root(runtime: &Harness, profile: &Path, source: &str) -> Option<PathBuf> {
    if *runtime != Harness::Pi {
        return package_name(source).then(|| {
            profile
                .join("profiles")
                .join(runtime.profile())
                .join("node_modules")
                .join(source)
        });
    }
    if let Some(spec) = source.strip_prefix("npm:") {
        let name = match spec.rfind('@') {
            Some(index) if index > 0 => &spec[..index],
            _ => spec,
        };
        return package_name(name).then(|| profile.join("npm/node_modules").join(name));
    }
    if source.starts_with("git:") || source.contains("://") {
        return git_checkout(profile, source);
    }
    if let Some(home_relative) = source.strip_prefix("~/") {
        return std::env::home_dir().map(|home| home.join(home_relative));
    }
    Some(pi_local_directory(profile, source))
}

pub(super) fn pi_local_directory(profile: &Path, source: &str) -> PathBuf {
    // Pi persists paths relative to its settings file and resolves them lexically with
    // Node path.resolve before opening. This matters when /var aliases /private/var.
    let mut result = PathBuf::new();
    for part in profile.join(source).components() {
        match part {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            _ => result.push(part),
        }
    }
    result
}

fn relative(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains(['\\', ':'])
        && !raw.chars().any(char::is_control)
        && Path::new(raw)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// Open from an already selected directory. npm/pnpm may symlink the package root itself.
// Each nested Unix component is opened relative to its parent descriptor, without following links.
#[cfg(unix)]
fn open_asset(root: &Path, entry: &Path) -> Option<fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
    };
    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)
        .ok()?;
    let mut components = entry
        .components()
        .filter(|c| *c != Component::CurDir)
        .peekable();
    while let Some(Component::Normal(name)) = components.next() {
        let name = CString::new(name.as_bytes()).ok()?;
        let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        if components.peek().is_some() {
            flags |= libc::O_DIRECTORY;
        }
        // SAFETY: the directory descriptor and NUL-terminated component live through the call.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return None;
        }
        // SAFETY: openat returned a new descriptor owned by this call.
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Some(directory)
}

#[cfg(windows)]
fn open_asset(root: &Path, entry: &Path) -> Option<fs::File> {
    use std::{
        ffi::OsString,
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let file = fs::File::open(root.join(entry)).ok()?;
    let handle = file.as_raw_handle();
    // SAFETY: the handle stays open and the second buffer has the size reported by Windows.
    let size = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0) };
    if size == 0 || size > 32768 {
        return None;
    }
    let mut buf = vec![0; size as usize];
    let len = unsafe { GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), size, 0) };
    if len == 0 || len >= size {
        return None;
    }
    let path = PathBuf::from(OsString::from_wide(&buf[..len as usize]));
    path.starts_with(root).then_some(file)
}

fn asset(root: &Path, entry: &Path, limit: u64) -> Option<fs::File> {
    let file = open_asset(root, entry)?;
    let metadata = file.metadata().ok()?;
    (metadata.is_file() && metadata.len() <= limit).then_some(file)
}

fn read(file: fs::File, limit: u64) -> Option<String> {
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

fn package_pages(runtime: &Harness, root: &Path) -> Vec<Page> {
    let Some(manifest) = asset(root, Path::new("package.json"), MAX_MANIFEST)
        .and_then(|file| read(file, MAX_MANIFEST))
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return vec![];
    };
    let Some(name) = manifest
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| package_name(n))
    else {
        return vec![];
    };
    if manifest
        .pointer("/nola/schema_version")
        .is_some_and(|v| v.as_u64() != Some(1))
    {
        return Vec::new();
    }
    let Some(pages) = manifest.pointer("/nola/pages").and_then(Value::as_array) else {
        return vec![];
    };
    pages
        .iter()
        .filter_map(|record| {
            let Declaration { id, title, entry } = serde_json::from_value(record.clone()).ok()?;
            if id.is_empty()
                || id.len() > 80
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            {
                return None;
            }
            let title = title.trim();
            if title.is_empty()
                || title.chars().count() > 120
                || title.chars().any(char::is_control)
                || !relative(&entry)
            {
                return None;
            }
            let entry = PathBuf::from(entry);
            if !entry.extension()?.eq_ignore_ascii_case("html") {
                return None;
            }
            asset(root, &entry, MAX_HTML)?;
            Some(Page {
                id: format!("{}:{}:{}", runtime.profile(), name, id),
                package: name.into(),
                title: title.into(),
                root: root.into(),
                entry,
            })
        })
        .collect()
}

const PROFILES: [Harness; 3] = [Harness::Pi, Harness::Dsh, Harness::DshWeb];

struct Package {
    source: String,
    pages: Vec<Page>,
}

fn packages(runtime: &Harness) -> Result<Vec<Package>, String> {
    let profile = runtime.root()?.join("profile");
    let mut id_counts = BTreeMap::new();
    let mut packages = Vec::new();
    for source in installed(runtime)? {
        let pages = package_root(runtime, &profile, &source)
            .and_then(|path| path.canonicalize().ok())
            .map(|root| package_pages(runtime, &root))
            .unwrap_or_default();
        for page in &pages {
            *id_counts.entry(page.id.clone()).or_insert(0) += 1;
        }
        packages.push(Package { source, pages });
    }
    // IDs are already profile-scoped. Suppress ambiguous declarations in all projections.
    for package in &mut packages {
        package.pages.retain(|page| id_counts[&page.id] == 1);
    }
    Ok(packages)
}

pub(crate) fn library(host: &str, local: bool) -> Value {
    let mut profiles = Vec::new();
    for runtime in PROFILES {
        let mut record = json!({"runtime":runtime.program(),"profile":runtime.profile(),
            "available":available(&runtime),"packages":[]});
        match packages(&runtime) {
            Ok(packages) => {
                let sources = packages
                    .iter()
                    .map(|item| item.source.clone())
                    .collect::<Vec<_>>();
                let mut links = BTreeMap::<String, Vec<Value>>::new();
                match super::pages(&runtime, &sources, host, local) {
                    Ok(entries) => {
                        for page in entries {
                            links
                                .entry(page.package)
                                .or_default()
                                .push(json!({"title":page.title,"url":page.url}));
                        }
                    }
                    Err(error) => record["error"] = json!(error),
                }
                record["packages"] = Value::Array(
                    packages
                        .into_iter()
                        .map(|package| {
                            let pages = package
                                .pages
                                .into_iter()
                                .map(|page| json!({"id":page.id,"title":page.title}))
                                .collect::<Vec<_>>();
                            let links = links.remove(&package.source).unwrap_or_default();
                            json!({"source":package.source,"pages":pages,"links":links})
                        })
                        .collect(),
                );
            }
            Err(error) => record["error"] = json!(error),
        }
        profiles.push(record);
    }
    json!({"profiles":profiles})
}

pub(crate) fn catalog(id: Option<&str>) -> Result<Value, String> {
    let mut all = Vec::new();
    for runtime in PROFILES {
        if let Some(id) = id {
            if id.split_once(':').map(|(owner, _)| owner) != Some(runtime.profile()) {
                continue;
            }
        }
        let packages = match packages(&runtime) {
            Ok(items) => items,
            Err(error) if id.is_some() => return Err(error),
            // A profile's read error is reported in the installed library. Healthy pages stay navigable.
            Err(_) => continue,
        };
        for page in packages.into_iter().flat_map(|package| package.pages) {
            if id == Some(&page.id) {
                let html = asset(&page.root, &page.entry, MAX_HTML)
                    .and_then(|file| read(file, MAX_HTML))
                    .ok_or("Cannot read desktop plugin page")?;
                return Ok(
                    json!({"id":page.id,"package":page.package,"title":page.title,"html":html}),
                );
            }
            if id.is_none() {
                all.push(page);
            }
        }
    }
    if id.is_some() {
        return Err("Desktop plugin page is unavailable".into());
    }
    all.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(json!(all))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_storage_paths_preserve_owners_and_ignore_versions() {
        let profile = Path::new("profile");
        for (source, relative) in [
            ("npm:@acme/board@2.1.0", "npm/node_modules/@acme/board"),
            ("npm:board@latest", "npm/node_modules/board"),
            ("git:github.com/acme/board@v2", "git/github.com/acme/board"),
            (
                "git:git@github.com:acme/board.git",
                "git/github.com/acme/board",
            ),
        ] {
            assert_eq!(
                package_root(&Harness::Pi, profile, source),
                Some(profile.join(relative))
            );
        }
        assert!(package_root(&Harness::Pi, profile, "npm:../other").is_none());
        assert!(package_root(&Harness::Dsh, profile, "../other").is_none());
        assert_eq!(
            package_root(&Harness::DshWeb, profile, "board"),
            Some(profile.join("profiles/web/node_modules/board"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_real_bounded_html_in_the_installed_package_is_visible() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("installed");
        fs::create_dir(&root).unwrap();
        fs::write(temp.path().join("outside.html"), "private").unwrap();
        fs::write(root.join("valid.html"), "hello").unwrap();
        fs::File::create(root.join("large.html"))
            .unwrap()
            .set_len(MAX_HTML + 1)
            .unwrap();
        symlink(temp.path().join("outside.html"), root.join("linked.html")).unwrap();
        let entries: Vec<_> = [
            "./valid.html",
            "linked.html",
            "../outside.html",
            "large.html",
            "missing.html",
        ]
        .iter()
        .enumerate()
        .map(|(n, entry)| json!({"id":n.to_string(),"title":"Board","entry":entry}))
        .collect();
        fs::write(
            root.join("package.json"),
            json!({"name":"test","nola":{"pages":entries}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            json!(package_pages(&Harness::Pi, &root)),
            json!([{"id":"pi:test:0","package":"test","title":"Board"}])
        );
        for version in [json!(2), json!(null), json!("1")] {
            fs::write(
                root.join("package.json"),
                json!({"name":"test","nola":{"schema_version":version,"pages":entries}})
                    .to_string(),
            )
            .unwrap();
            assert!(package_pages(&Harness::Pi, &root).is_empty());
        }
    }
}
