#![windows_subsystem = "windows"]

mod app;
mod clip;
mod config;
mod history;
mod hook;
mod paint;
mod preview;
mod settings;
mod store;
mod tray;
mod util;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn main() {
    unsafe {
        // Single instance: the mutex lives until the process exits.
        let name = util::wide("Local\\UnshufflzSingleInstance");
        let m = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if m.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            // Already running: ask that instance to open its settings window.
            let h = FindWindowW(util::wide("UnshufflzOverlay").as_ptr(), std::ptr::null());
            if !h.is_null() {
                // Let it bring the settings window to the front.
                let mut pid = 0;
                GetWindowThreadProcessId(h, &mut pid);
                AllowSetForegroundWindow(pid);
                PostMessageW(h, app::WM_APP_SETTINGS, 0, 0);
            }
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    app::run(config::load());
}
