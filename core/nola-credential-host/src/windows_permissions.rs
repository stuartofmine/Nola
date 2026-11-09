// SPDX-License-Identifier: Apache-2.0
//! Current-token-only protected DACLs for private files and registry records.
use std::path::Path;
use windows_sys::Win32::{Foundation::{CloseHandle, LocalFree, HANDLE}, Security::*,
    Security::Authorization::*, System::Threading::{GetCurrentProcess, OpenProcessToken}};

struct Token(HANDLE);
impl Drop for Token { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
struct Descriptor { pointer: PSECURITY_DESCRIPTOR, sid: String }
impl Drop for Descriptor { fn drop(&mut self) { unsafe { LocalFree(self.pointer); } } }

unsafe fn sid_text(sid: PSID) -> Result<String, String> {
    let mut text = std::ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut text) == 0 { return Err("Cannot encode private object owner".into()); }
    let mut count = 0;
    while count < 256 && *text.add(count) != 0 { count += 1; }
    let result = String::from_utf16(std::slice::from_raw_parts(text, count));
    LocalFree(text.cast());
    result.map_err(|_| "Invalid private object owner".into())
}

fn descriptor(inherit: bool, registry: bool) -> Result<Descriptor, String> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 { return Err("Cannot identify private object owner".into()); }
    let token = Token(token);
    let mut length = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut length); }
    if length == 0 || length > 65536 { return Err("Invalid private object owner".into()); }
    let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), length, &mut length) } == 0 { return Err("Cannot inspect private object owner".into()); }
    let owner = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let sid = unsafe { sid_text(owner.User.Sid)? };
    let flags = if inherit { "OICI" } else { "" };
    let access = if registry { "KA" } else { "FA" };
    let text: Vec<u16> = format!("D:P(A;{flags};{access};;;{sid})\0").encode_utf16().collect();
    let mut pointer = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(), 1, &mut pointer, std::ptr::null_mut()) } == 0 { return Err("Cannot build private object permissions".into()); }
    Ok(Descriptor { pointer, sid })
}

unsafe fn validate_owner(owner: PSID, descriptor: PSECURITY_DESCRIPTOR, expected: &str) -> Result<(), String> {
    // Administrators/SYSTEM already have OS-level takeover authority. An
    // unrelated ordinary owner must not retain the ability to rewrite the DACL.
    let allowed = sid_text(owner).map(|value| value == expected).unwrap_or(false)
        || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
        || IsWellKnownSid(owner, WinLocalSystemSid) != 0;
    LocalFree(descriptor);
    if allowed { Ok(()) } else { Err("Private object has another owner".into()) }
}

pub fn protect_private_path(path: &Path) -> Result<(), String> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "Cannot inspect private path")?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 { return Err("Private path cannot be a reparse point".into()); }
    let desired = descriptor(metadata.is_dir(), false)?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let (mut owner, mut current) = (std::ptr::null_mut(), std::ptr::null_mut());
        if GetNamedSecurityInfoW(path.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION,
            &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut current) != 0 {
            return Err("Cannot validate private path owner".into());
        }
        validate_owner(owner, current, &desired.sid)?;
        if SetFileSecurityW(path.as_ptr(), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, desired.pointer) == 0 {
            return Err("Cannot protect private path permissions".into());
        }
    }
    Ok(())
}

pub fn protect_registry_key(key: windows_sys::Win32::System::Registry::HKEY) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::RegSetKeySecurity;
    let desired = descriptor(false, true)?;
    unsafe {
        let (mut owner, mut current) = (std::ptr::null_mut(), std::ptr::null_mut());
        if GetSecurityInfo(key, SE_REGISTRY_KEY, OWNER_SECURITY_INFORMATION,
            &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut current) != 0 {
            return Err("Cannot validate private registry owner".into());
        }
        validate_owner(owner, current, &desired.sid)?;
        if RegSetKeySecurity(key, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, desired.pointer) != 0 {
            return Err("Cannot protect input journal permissions".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_current_principal_can_reopen_files_and_inheritance_is_removed() {
        use std::os::windows::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        protect_private_path(directory.path()).unwrap();
        let path = directory.path().join("private.txt");
        std::fs::write(&path, "fixture").unwrap();
        protect_private_path(&path).unwrap();
        std::fs::write(&path, "updated").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated");
        let native:Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut acl = std::ptr::null_mut();
        let mut current = std::ptr::null_mut();
        unsafe {
            assert_eq!(GetNamedSecurityInfoW(native.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(), std::ptr::null_mut(), &mut acl, std::ptr::null_mut(), &mut current), 0);
            assert!(!acl.is_null());
            assert_eq!((*acl).AceCount, 1);
            let mut control = 0;
            let mut revision = 0;
            assert_ne!(GetSecurityDescriptorControl(current, &mut control, &mut revision), 0);
            assert_ne!(control & SE_DACL_PROTECTED, 0);
            LocalFree(current);
        }
    }
}
