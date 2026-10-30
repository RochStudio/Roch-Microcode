use std::path::PathBuf;

/// Local data folder.
///
/// Continue using an existing data folder so renaming the product does not
/// strand theme settings, verified tool caches, or USB-file backups:
/// `%LOCALAPPDATA%/Roch Microcode`, or the legacy `%LOCALAPPDATA%/Roch BIOS`
/// when that folder exists and the new one does not.
pub fn data_directory() -> PathBuf {
    let local = local_app_data();
    let current = local.join("Roch Microcode");
    let legacy = local.join("Roch BIOS");
    if legacy.is_dir() && !current.is_dir() {
        legacy
    } else {
        current
    }
}

#[cfg(windows)]
fn local_app_data() -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};
    // Same source as Environment.GetFolderPath(LocalApplicationData).
    // SAFETY: SHGetKnownFolderPath returns a NUL-terminated CoTaskMem string that we free.
    unsafe {
        let mut raw: *mut u16 = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, std::ptr::null_mut(), &mut raw);
        if hr >= 0 && !raw.is_null() {
            let mut len = 0usize;
            while *raw.add(len) != 0 {
                len += 1;
            }
            let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw, len));
            CoTaskMemFree(raw as _);
            return PathBuf::from(path);
        }
        if !raw.is_null() {
            CoTaskMemFree(raw as _);
        }
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[cfg(not(windows))]
fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

/// A new unique job folder under `verification-jobs`.
pub fn new_verification_job() -> PathBuf {
    data_directory().join("verification-jobs").join(crate::format::guid_n())
}
