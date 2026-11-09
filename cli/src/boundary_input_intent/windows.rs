// SPDX-License-Identifier: Apache-2.0
//! Windows durable client journal. RegFlushKey persists namespace and value
//! updates without pretending a directory FlushFileBuffers is supported.
use windows_sys::Win32::{Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS}, System::Registry::*};

pub(super) struct RegistryJournal { key: HKEY, value: Vec<u16> }
impl Drop for RegistryJournal { fn drop(&mut self) { unsafe { RegCloseKey(self.key); } } }
fn error(code: u32) -> String { format!("Input journal registry operation failed: {}", std::io::Error::from_raw_os_error(code as i32)) }
impl RegistryJournal {
    pub(super) fn open(value: &str) -> Result<Self, String> {
        #[cfg(not(test))]
        let namespace = "Software\\Nola\\ClientInputIntents".to_string();
        #[cfg(test)]
        let namespace = {
            static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            format!("Software\\Nola\\Tests\\{}\\ClientInputIntents", OWNER.get_or_init(|| uuid::Uuid::new_v4().to_string()))
        };
        let namespace:Vec<u16> = namespace.encode_utf16().chain(Some(0)).collect();
        let mut key = std::ptr::null_mut();
        let result = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, namespace.as_ptr(), 0, std::ptr::null(),
            REG_OPTION_NON_VOLATILE, KEY_QUERY_VALUE | KEY_SET_VALUE | windows_sys::Win32::Storage::FileSystem::WRITE_DAC | windows_sys::Win32::Storage::FileSystem::READ_CONTROL,
            std::ptr::null(), &mut key, std::ptr::null_mut()) };
        if result != ERROR_SUCCESS { return Err(error(result)); }
        let journal = Self { key, value: value.encode_utf16().chain(Some(0)).collect() };
        nola_credential_host::windows_permissions::protect_registry_key(key)?;
        Ok(journal)
    }
    pub(super) fn read(&self, maximum: usize) -> Result<Option<Vec<u8>>, String> {
        let (mut kind, mut length) = (0, 0);
        let result = unsafe { RegQueryValueExW(self.key, self.value.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut length) };
        if result == ERROR_FILE_NOT_FOUND { return Ok(None); }
        if result != ERROR_SUCCESS { return Err(error(result)); }
        if kind != REG_BINARY || length as usize > maximum { return Err("Invalid or oversized input journal value".into()); }
        let mut bytes = vec![0; length as usize];
        let result = unsafe { RegQueryValueExW(self.key, self.value.as_ptr(), std::ptr::null(), &mut kind, bytes.as_mut_ptr(), &mut length) };
        if result != ERROR_SUCCESS { return Err(error(result)); }
        if kind != REG_BINARY || length as usize > bytes.len() { return Err("Input journal changed while reading".into()); }
        bytes.truncate(length as usize);
        self.flush()?;
        Ok(Some(bytes))
    }
    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let result = unsafe { RegSetValueExW(self.key, self.value.as_ptr(), 0, REG_BINARY, bytes.as_ptr(), bytes.len().try_into().map_err(|_| "Input journal too large")?) };
        if result != ERROR_SUCCESS { return Err(error(result)); }
        self.flush()
    }
    pub(super) fn unlink(&self) -> Result<(), String> {
        let result = unsafe { RegDeleteValueW(self.key, self.value.as_ptr()) };
        if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND { return Err(error(result)); }
        Ok(())
    }
    pub(super) fn clear(&self) -> Result<(), String> { self.unlink()?; self.flush() }
    fn flush(&self) -> Result<(), String> {
        let result = unsafe { RegFlushKey(self.key) };
        if result == ERROR_SUCCESS { Ok(()) } else { Err(error(result)) }
    }
}
