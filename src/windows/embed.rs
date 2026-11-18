//! A plain Win32 child window inside the main window. mpv draws into it (`--wid`),
//! and we move/resize it to match the TV screen, and hide it during static.

use eframe::egui::{Rect, Ui};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow,
    WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED,
};

pub struct VideoWindow {
    hwnd: HWND,
    visible: bool,
    geom: (i32, i32, u32, u32),
}

/// The main window's HWND.
pub fn parent(cc: &eframe::CreationContext<'_>) -> Result<isize, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    cc.window_handle()
        .ok()
        .and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        })
        .ok_or_else(|| "Couldn't find the main window.".to_string())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl VideoWindow {
    pub fn new(parent: isize) -> Result<Self, String> {
        let class = wide("STATIC");
        let empty = wide("");
        // SAFETY: plain Win32 call; strings are NUL-terminated and outlive it.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                empty.as_ptr(),
                // Disabled: clicks and keys fall through to our window.
                WS_CHILD | WS_CLIPSIBLINGS | WS_DISABLED,
                0,
                0,
                16,
                16,
                parent as HWND,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self { hwnd, visible: false, geom: (0, 0, 16, 16) })
    }

    /// What the player draws into (mpv takes the HWND as a number).
    pub fn handle(&self) -> isize {
        self.hwnd as isize
    }

    pub fn prepare(&mut self, _frame: &mut eframe::Frame) {}

    pub fn show(&mut self, ui: &Ui, screen: Rect, visible: bool) {
        let ppp = ui.ctx().pixels_per_point();
        let g = (
            (screen.min.x * ppp).round() as i32,
            (screen.min.y * ppp).round() as i32,
            ((screen.width() * ppp).round() as u32).max(1),
            ((screen.height() * ppp).round() as u32).max(1),
        );
        if g != self.geom {
            self.geom = g;
            // SAFETY: our own window handle.
            unsafe {
                SetWindowPos(self.hwnd, std::ptr::null_mut(), g.0, g.1, g.2 as i32, g.3 as i32, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        if visible != self.visible {
            self.visible = visible;
            // SAFETY: our own window handle.
            unsafe {
                ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            }
        }
    }
}

impl Drop for VideoWindow {
    fn drop(&mut self) {
        // SAFETY: our own window handle, destroyed once.
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
