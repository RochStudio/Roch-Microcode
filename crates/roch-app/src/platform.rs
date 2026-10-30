//! Thin Win32 helpers: title-bar colours, message boxes, shell open, cursor
//! position for drag-and-drop targeting, CPUID and the registry microcode value.

use egui::Color32;

/// Window handle of the main viewport (0 when unavailable).
pub type Hwnd = isize;

fn hwnd_from(handle: &impl raw_window_handle::HasWindowHandle) -> Hwnd {
    use raw_window_handle::RawWindowHandle;
    match handle.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

pub fn hwnd_of(frame: &eframe::Frame) -> Hwnd {
    hwnd_from(frame)
}

pub fn hwnd_of_creation(cc: &eframe::CreationContext<'_>) -> Hwnd {
    hwnd_from(cc)
}

fn colorref(c: Color32) -> u32 {
    c.r() as u32 | (c.g() as u32) << 8 | (c.b() as u32) << 16
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Match the native title bar to the active theme. Caption/text/border colours
/// are supported on Windows 11; older versions reject these attributes and
/// retain their native frame behaviour.
#[cfg(windows)]
pub fn apply_title_bar(hwnd: Hwnd, light: bool, canvas: Color32, text: Color32, outline: Color32) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    if hwnd == 0 {
        return;
    }
    let set = |attribute: i32, value: u32| {
        // SAFETY: hwnd is our live top-level window; value points to a 4-byte DWORD.
        unsafe {
            DwmSetWindowAttribute(hwnd as _, attribute as _, &value as *const u32 as *const _, 4);
        }
    };
    set(20, if light { 0 } else { 1 }); // DWMWA_USE_IMMERSIVE_DARK_MODE
    set(35, colorref(canvas)); // DWMWA_CAPTION_COLOR
    set(36, colorref(text)); // DWMWA_TEXT_COLOR
    set(34, colorref(outline)); // DWMWA_BORDER_COLOR
}

#[cfg(not(windows))]
pub fn apply_title_bar(_hwnd: Hwnd, _light: bool, _canvas: Color32, _text: Color32, _outline: Color32) {}

/// Cursor position in the window's client area, in physical pixels.
/// Used to find the card a file was dropped on (winit reports no drop position).
#[cfg(windows)]
pub fn cursor_client_pos(hwnd: Hwnd) -> Option<(f32, f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    if hwnd == 0 {
        return None;
    }
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: valid out-pointer and window handle.
    unsafe {
        if GetCursorPos(&mut p) == 0 || ScreenToClient(hwnd as _, &mut p) == 0 {
            return None;
        }
    }
    Some((p.x as f32, p.y as f32))
}

#[cfg(not(windows))]
pub fn cursor_client_pos(_hwnd: Hwnd) -> Option<(f32, f32)> {
    None
}

/// Modal warning (`MessageBox.Show(..., Warning)`).
#[cfg(windows)]
pub fn warning_box(hwnd: Hwnd, title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
    let (t, c) = (wide(text), wide(title));
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    unsafe {
        MessageBoxW(hwnd as _, t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

#[cfg(not(windows))]
pub fn warning_box(_hwnd: Hwnd, title: &str, text: &str) {
    eprintln!("{title}: {text}");
}

/// Yes/No question with **No** as the default button.
#[cfg(windows)]
pub fn confirm_box(hwnd: Hwnd, title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_YESNO};
    let (t, c) = (wide(text), wide(title));
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    unsafe { MessageBoxW(hwnd as _, t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES }
}

#[cfg(not(windows))]
pub fn confirm_box(_hwnd: Hwnd, _title: &str, _text: &str) -> bool {
    false
}

/// Open a URL or folder with the default handler (`UseShellExecute = true`).
#[cfg(windows)]
pub fn shell_open(target: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, file) = (wide("open"), wide(target));
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(not(windows))]
pub fn shell_open(target: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open").arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// EAX of CPUID leaf 1 (processor signature), or 0 when unavailable.
pub fn cpuid_signature() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        // CPUID is available on every x86-64 processor.
        std::arch::x86_64::__cpuid(1).eax
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// `HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0` "Update Revision" (bytes 4..8).
#[cfg(windows)]
pub fn windows_microcode_revision() -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
    let key = wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
    let value = wide("Update Revision");
    let mut data = [0u8; 64];
    let mut size = data.len() as u32;
    // SAFETY: buffers are valid for the given sizes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            data.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    if status == 0 && size >= 8 {
        Some(u32::from_le_bytes([data[4], data[5], data[6], data[7]]))
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn windows_microcode_revision() -> Option<u32> {
    None
}

/// `Environment.GetFolderPath(SpecialFolder.System)`.
#[cfg(windows)]
pub fn system_directory() -> std::path::PathBuf {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = [0u16; 520];
    // SAFETY: buffer length is passed correctly.
    let n = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if n > 0 && n < buffer.len() {
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..n]))
    } else {
        std::path::PathBuf::from(r"C:\Windows\System32")
    }
}

#[cfg(not(windows))]
pub fn system_directory() -> std::path::PathBuf {
    std::path::PathBuf::from("/usr/bin")
}

/// Desktop work area in logical points (system DPI), for the initial window size.
#[cfg(windows)]
pub fn work_area_points() -> Option<(f32, f32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: SPI_GETWORKAREA writes one RECT.
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as *mut _, 0) };
    if ok == 0 {
        return None;
    }
    // SAFETY: no arguments.
    let dpi = unsafe { GetDpiForSystem() }.max(96) as f32;
    let scale = dpi / 96.0;
    Some(((r.right - r.left) as f32 / scale, (r.bottom - r.top) as f32 / scale))
}

#[cfg(not(windows))]
pub fn work_area_points() -> Option<(f32, f32)> {
    None
}
