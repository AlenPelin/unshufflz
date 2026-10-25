//! Settings window (tray → Settings…): overlay width, history size, show delay, left/right placement.
//! Plain Win32 controls; themed via the Common Controls v6 manifest.

use crate::app;
use crate::config::{Side, MAX_ITEMS_LIMIT};
use crate::util::wide;
use std::cell::Cell;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_TRACK: usize = 100;
const ID_RIGHT: usize = 101;
const ID_LEFT: usize = 102;
const ID_COUNT: usize = 103;
const ID_DELAY: usize = 104;

const TBM_GETPOS: u32 = WM_USER;
const TBM_SETPOS: u32 = WM_USER + 5;
const TBM_SETRANGEMIN: u32 = WM_USER + 7;
const TBM_SETRANGEMAX: u32 = WM_USER + 8;
const TBM_SETTICFREQ: u32 = WM_USER + 20;
const TBM_SETPAGESIZE: u32 = WM_USER + 21;
const TBS_AUTOTICKS: u32 = 1;
const BS_PUSHBUTTON_: u32 = 0;
const BS_DEFPUSHBUTTON_: u32 = 1;
const BS_AUTORADIOBUTTON_: u32 = 9;
const SS_RIGHT_: u32 = 2;
const ES_NUMBER_: u32 = 0x2000;
const ES_RIGHT_: u32 = 2;
const EM_SETLIMITTEXT_: u32 = 0xC5;
const BST_CHECKED_: usize = 1;

const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;

struct Ui {
    hwnd: HWND,
    track: HWND,
    value: HWND,
    right: HWND,
    count: HWND,
    delay: HWND,
    font: HFONT,
}

thread_local! {
    static UI: Cell<Option<&'static Ui>> = const { Cell::new(None) };
}

/// The open settings window, or null. Used by the message loop for IsDialogMessageW.
pub fn hwnd() -> HWND {
    UI.with(|u| u.get().map_or(null_mut(), |u| u.hwnd))
}

/// Open the window (or bring it to front) with the current values.
pub fn open(width_pct: u32, side: Side, max_items: usize, hold_ms: u32) {
    unsafe {
        let h = hwnd();
        if !h.is_null() {
            SetForegroundWindow(h);
            return;
        }
        let icc = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_BAR_CLASSES };
        InitCommonControlsEx(&icc);
        let inst = GetModuleHandleW(null());
        let class = wide("UnshufflzSettings");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc); // fails harmlessly when already registered
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("Unshufflz Settings").as_ptr(), STYLE, CW_USEDEFAULT, CW_USEDEFAULT, 100, 100, null_mut(), null_mut(), inst, null());
        if hwnd.is_null() {
            return;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = |v: i32| v * dpi as i32 / 96;

        // Size and center on the monitor under the cursor.
        let mut r = RECT { left: 0, top: 0, right: s(360), bottom: s(276) };
        AdjustWindowRectExForDpi(&mut r, STYLE, 0, 0, dpi);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut pt = std::mem::zeroed();
        GetCursorPos(&mut pt);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
        let wa = mi.rcWork;
        SetWindowPos(hwnd, null_mut(), wa.left + (wa.right - wa.left - ww) / 2, wa.top + (wa.bottom - wa.top - wh) / 2, ww, wh, SWP_NOZORDER);

        // Dialog font for this DPI.
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut _, 0, dpi);
        let font = CreateFontIndirectW(&ncm.lfMessageFont);

        let child = |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| {
            let ex = if class == "EDIT" { WS_EX_CLIENTEDGE } else { 0 };
            let c = CreateWindowExW(ex, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | WS_VISIBLE | style, s(x), s(y), s(w), s(h), hwnd, id as HMENU, inst, null());
            SendMessageW(c, WM_SETFONT, font as WPARAM, 1);
            c
        };
        child("STATIC", "Overlay width (% of screen width)", 0, 16, 16, 260, 20, 0);
        let value = child("STATIC", "", SS_RIGHT_, 276, 16, 68, 20, 0);
        let track = child("msctls_trackbar32", "", WS_TABSTOP | TBS_AUTOTICKS, 10, 40, 340, 30, ID_TRACK);
        SendMessageW(track, TBM_SETRANGEMIN, 0, 5);
        SendMessageW(track, TBM_SETRANGEMAX, 0, 50);
        SendMessageW(track, TBM_SETTICFREQ, 5, 0);
        SendMessageW(track, TBM_SETPAGESIZE, 0, 5);
        SendMessageW(track, TBM_SETPOS, 1, width_pct as LPARAM);
        child("STATIC", &format!("Items to keep in history (1-{})", MAX_ITEMS_LIMIT), 0, 16, 88, 260, 20, 0);
        let count = child("EDIT", &max_items.to_string(), WS_TABSTOP | ES_NUMBER_ | ES_RIGHT_, 284, 85, 60, 24, ID_COUNT);
        SendMessageW(count, EM_SETLIMITTEXT_, 3, 0);
        child("STATIC", "Show delay when holding Win (ms, 0-2000)", 0, 16, 122, 260, 20, 0);
        let delay = child("EDIT", &hold_ms.to_string(), WS_TABSTOP | ES_NUMBER_ | ES_RIGHT_, 284, 119, 60, 24, ID_DELAY);
        SendMessageW(delay, EM_SETLIMITTEXT_, 4, 0);
        child("STATIC", "Position", 0, 16, 152, 260, 20, 0);
        let right = child("BUTTON", "Right side of the screen", WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON_, 24, 174, 300, 22, ID_RIGHT);
        let left = child("BUTTON", "Left side of the screen", BS_AUTORADIOBUTTON_, 24, 198, 300, 22, ID_LEFT);
        SendMessageW(if side == Side::Left { left } else { right }, BM_SETCHECK, BST_CHECKED_, 0);
        child("BUTTON", "OK", WS_TABSTOP | WS_GROUP | BS_DEFPUSHBUTTON_, 168, 236, 84, 28, IDOK as usize);
        child("BUTTON", "Cancel", WS_TABSTOP | BS_PUSHBUTTON_, 260, 236, 84, 28, IDCANCEL as usize);

        let ui: &'static Ui = Box::leak(Box::new(Ui { hwnd, track, value, right, count, delay, font }));
        UI.with(|u| u.set(Some(ui)));
        update_value();
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(track);
    }
}

fn update_value() {
    UI.with(|u| {
        if let Some(ui) = u.get() {
            unsafe {
                let v = SendMessageW(ui.track, TBM_GETPOS, 0, 0);
                SetWindowTextW(ui.value, wide(&format!("{} %", v)).as_ptr());
            }
        }
    });
}

fn apply() {
    let Some(ui) = UI.with(|u| u.get()) else { return };
    unsafe {
        let width = SendMessageW(ui.track, TBM_GETPOS, 0, 0).clamp(5, 50) as u32;
        let side = if SendMessageW(ui.right, BM_GETCHECK, 0, 0) as usize == BST_CHECKED_ { Side::Right } else { Side::Left };
        let number = |h: HWND| {
            let mut buf = [0u16; 8];
            let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            String::from_utf16_lossy(&buf[..n]).trim().parse::<u32>().ok()
        };
        let count = number(ui.count).unwrap_or(10).clamp(1, MAX_ITEMS_LIMIT as u32) as usize;
        let delay = number(ui.delay).unwrap_or(0).min(2000);
        app::apply_settings(width, side, count, delay);
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_HSCROLL => {
            update_value();
            0
        }
        WM_COMMAND => {
            match (wp & 0xFFFF) as i32 {
                IDOK => {
                    apply();
                    DestroyWindow(h);
                }
                IDCANCEL => {
                    DestroyWindow(h);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(h);
            0
        }
        WM_DESTROY => {
            if let Some(ui) = UI.with(|u| u.take()) {
                DeleteObject(ui.font);
                drop(Box::from_raw(ui as *const Ui as *mut Ui));
            }
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}
