use crate::util::wide;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_TRAY: u32 = WM_APP + 3;
pub const ID_CLEAR: u32 = 1;
pub const ID_AUTOSTART: u32 = 2;
pub const ID_EXIT: u32 = 3;
pub const ID_SETTINGS: u32 = 4;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Unshufflz";

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d
}

pub fn add(hwnd: HWND) {
    let mut d = data(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    d.uCallbackMessage = WM_APP_TRAY;
    d.hIcon = make_icon();
    for (i, c) in "Unshufflz (hold Win: V back, C forward, B delete, S save image)".encode_utf16().take(127).enumerate() {
        d.szTip[i] = c;
    }
    unsafe { Shell_NotifyIconW(NIM_ADD, &d) };
}

pub fn remove(hwnd: HWND) {
    let d = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
}

/// Small clipboard picture drawn into a 32-bit icon.
fn make_icon() -> HICON {
    unsafe {
        let n = GetSystemMetrics(SM_CXSMICON).max(16);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = n;
        bmi.bmiHeader.biHeight = -n;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let color = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() {
            return LoadIconW(null_mut(), IDI_APPLICATION);
        }
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (n * n) as usize);
        let f = n as f32;
        let inside = |x: i32, y: i32, x0: f32, y0: f32, x1: f32, y1: f32| {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            x >= x0 * f && x < x1 * f && y >= y0 * f && y < y1 * f
        };
        for y in 0..n {
            for x in 0..n {
                px[(y * n + x) as usize] = if inside(x, y, 0.34, 0.03, 0.66, 0.2) {
                    0xFF50_5050 // clip
                } else if inside(x, y, 0.25, 0.25, 0.75, 0.86) {
                    0xFFFF_FFFF // paper
                } else if inside(x, y, 0.13, 0.1, 0.87, 0.97) {
                    0xFF00_78D7 // board
                } else {
                    0
                };
            }
        }
        let mask = CreateBitmap(n, n, 1, 1, null());
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

/// Show the tray menu; returns the chosen command id (0 = none).
pub fn menu(hwnd: HWND) -> u32 {
    unsafe {
        let m = CreatePopupMenu();
        AppendMenuW(m, MF_STRING, ID_SETTINGS as usize, wide("Settings…").as_ptr());
        AppendMenuW(m, MF_STRING, ID_CLEAR as usize, wide("Clear history").as_ptr());
        let flags = MF_STRING | if autostart_enabled() { MF_CHECKED } else { 0 };
        AppendMenuW(m, flags, ID_AUTOSTART as usize, wide("Start with Windows").as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        AppendMenuW(m, MF_STRING, ID_EXIT as usize, wide("Exit").as_ptr());
        let mut pt = std::mem::zeroed();
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        cmd as u32
    }
}

pub fn autostart_enabled() -> bool {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size) == ERROR_SUCCESS
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let cmd = wide(&format!("\"{}\"", crate::util::exe_path().display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr(), REG_SZ, cmd.as_ptr() as *const _, (cmd.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr());
        }
    }
}
