//! Settings window (tray → Settings…): overlay width, history size, show delay,
//! show with Start menu, folder for saved pictures, left/right placement.
//! Plain Win32 controls; themed via the Common Controls v6 manifest.

use crate::app;
use crate::config::{Config, Side, MAX_ITEMS_LIMIT};
use crate::imagesave;
use crate::util::wide;
use std::cell::Cell;
use std::path::Path;
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
const ID_PICS: usize = 105;
const ID_BROWSE: usize = 106;
const ID_START: usize = 107;

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
const BS_AUTOCHECKBOX_: u32 = 3;
const SS_RIGHT_: u32 = 2;
const ES_NUMBER_: u32 = 0x2000;
const ES_RIGHT_: u32 = 2;
const ES_AUTOHSCROLL_: u32 = 0x80;
const EM_SETLIMITTEXT_: u32 = 0xC5;
const EM_SETCUEBANNER_: u32 = 0x1501;
const BST_CHECKED_: usize = 1;

const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;

struct Ui {
    hwnd: HWND,
    track: HWND,
    value: HWND,
    right: HWND,
    count: HWND,
    delay: HWND,
    pics: HWND,
    start: HWND,
    font: HFONT,
    /// Config the window was opened with; OK changes only the fields shown here.
    base: Config,
}

thread_local! {
    static UI: Cell<Option<&'static Ui>> = const { Cell::new(None) };
}

/// The open settings window, or null. Used by the message loop for IsDialogMessageW.
pub fn hwnd() -> HWND {
    UI.with(|u| u.get().map_or(null_mut(), |u| u.hwnd))
}

/// Open the window (or bring it to front) with the current values.
pub fn open(cfg: Config) {
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
        let mut r = RECT { left: 0, top: 0, right: s(360), bottom: s(362) };
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
        SendMessageW(track, TBM_SETPOS, 1, cfg.width_pct as LPARAM);
        child("STATIC", &format!("Items to keep in history (1-{})", MAX_ITEMS_LIMIT), 0, 16, 88, 260, 20, 0);
        let count = child("EDIT", &cfg.max_items.to_string(), WS_TABSTOP | ES_NUMBER_ | ES_RIGHT_, 284, 85, 60, 24, ID_COUNT);
        SendMessageW(count, EM_SETLIMITTEXT_, 3, 0);
        child("STATIC", "Show delay when holding Win (ms, 0-2000)", 0, 16, 122, 260, 20, 0);
        let delay = child("EDIT", &cfg.hold_ms.to_string(), WS_TABSTOP | ES_NUMBER_ | ES_RIGHT_, 284, 119, 60, 24, ID_DELAY);
        SendMessageW(delay, EM_SETLIMITTEXT_, 4, 0);
        let start = child("BUTTON", "Also show while the Start menu is open", WS_TABSTOP | BS_AUTOCHECKBOX_, 16, 152, 328, 22, ID_START);
        if cfg.show_with_start {
            SendMessageW(start, BM_SETCHECK, BST_CHECKED_, 0);
        }
        child("STATIC", "Save pictures (Win+S) to folder", 0, 16, 186, 328, 20, 0);
        let pics = child("EDIT", &cfg.pictures_dir, WS_TABSTOP | ES_AUTOHSCROLL_, 16, 208, 240, 24, ID_PICS);
        // Empty box = default folder, shown as a grey hint.
        let hint = wide(&imagesave::default_dir().to_string_lossy());
        SendMessageW(pics, EM_SETCUEBANNER_, 1, hint.as_ptr() as LPARAM);
        child("BUTTON", "Browse…", WS_TABSTOP | BS_PUSHBUTTON_, 262, 207, 82, 26, ID_BROWSE);
        child("STATIC", "Position", 0, 16, 242, 260, 20, 0);
        let right = child("BUTTON", "Right side of the screen", WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON_, 24, 264, 300, 22, ID_RIGHT);
        let left = child("BUTTON", "Left side of the screen", BS_AUTORADIOBUTTON_, 24, 288, 300, 22, ID_LEFT);
        SendMessageW(if cfg.side == Side::Left { left } else { right }, BM_SETCHECK, BST_CHECKED_, 0);
        child("BUTTON", "OK", WS_TABSTOP | WS_GROUP | BS_DEFPUSHBUTTON_, 168, 322, 84, 28, IDOK as usize);
        child("BUTTON", "Cancel", WS_TABSTOP | BS_PUSHBUTTON_, 260, 322, 84, 28, IDCANCEL as usize);

        let ui: &'static Ui = Box::leak(Box::new(Ui { hwnd, track, value, right, count, delay, pics, start, font, base: cfg }));
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

fn text_of(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

/// Folder box value as stored in the config: trimmed, surrounding quotes removed.
/// Err if it is not empty and not a full path.
pub fn clean_dir(s: &str) -> Result<String, ()> {
    let s = s.trim().trim_matches('"').trim().to_string();
    if s.is_empty() || Path::new(&s).is_absolute() {
        Ok(s)
    } else {
        Err(())
    }
}

/// Read the controls and apply. Returns false (window stays open) if the folder is invalid.
fn apply() -> bool {
    let Some(ui) = UI.with(|u| u.get()) else { return true };
    unsafe {
        let Ok(pictures_dir) = clean_dir(&text_of(ui.pics)) else {
            MessageBoxW(ui.hwnd, wide("Please enter a full folder path (for example D:\\Screenshots), or leave the box empty for the default folder.").as_ptr(), wide("Unshufflz Settings").as_ptr(), MB_OK | MB_ICONWARNING);
            SetFocus(ui.pics);
            return false;
        };
        let number = |h: HWND| text_of(h).trim().parse::<u32>().ok();
        let mut cfg = ui.base.clone();
        cfg.width_pct = SendMessageW(ui.track, TBM_GETPOS, 0, 0).clamp(5, 50) as u32;
        cfg.side = if SendMessageW(ui.right, BM_GETCHECK, 0, 0) as usize == BST_CHECKED_ { Side::Right } else { Side::Left };
        cfg.max_items = number(ui.count).unwrap_or(10).clamp(1, MAX_ITEMS_LIMIT as u32) as usize;
        cfg.hold_ms = number(ui.delay).unwrap_or(0).min(2000);
        cfg.pictures_dir = pictures_dir;
        cfg.show_with_start = SendMessageW(ui.start, BM_GETCHECK, 0, 0) as usize == BST_CHECKED_;
        app::apply_settings(cfg);
    }
    true
}

/// Browse… button: modern folder picker, starting at the current (or default) folder.
fn browse() {
    let Some(ui) = UI.with(|u| u.get()) else { return };
    let current = clean_dir(&text_of(ui.pics)).unwrap_or_default();
    let start = if current.is_empty() { imagesave::default_dir() } else { current.into() };
    if let Some(dir) = pick_folder(ui.hwnd, &start) {
        unsafe { SetWindowTextW(ui.pics, wide(&dir).as_ptr()) };
    }
}

fn pick_folder(owner: HWND, start: &Path) -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND as WHWND;
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST).ok()?;
        let _ = dlg.SetTitle(&HSTRING::from("Folder for saved pictures"));
        // Start in the nearest existing folder.
        let mut dir = Some(start);
        while let Some(d) = dir {
            if d.is_dir() {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(d.to_string_lossy().as_ref()), None::<&IBindCtx>) {
                    let _ = dlg.SetFolder(&item);
                }
                break;
            }
            dir = d.parent();
        }
        dlg.Show(WHWND(owner as _)).ok()?; // Err when cancelled
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_HSCROLL => {
            update_value();
            0
        }
        WM_COMMAND => {
            match wp & 0xFFFF {
                id if id == IDOK as usize => {
                    if apply() {
                        DestroyWindow(h);
                    }
                }
                id if id == IDCANCEL as usize => {
                    DestroyWindow(h);
                }
                ID_BROWSE => browse(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_box_values() {
        assert_eq!(clean_dir("  "), Ok(String::new()));
        assert_eq!(clean_dir(r#" "D:\Shots" "#), Ok(r"D:\Shots".to_string()));
        assert_eq!(clean_dir(r"\\server\share\pics"), Ok(r"\\server\share\pics".to_string()));
        assert!(clean_dir("relative\\dir").is_err());
        assert!(clean_dir(r"D:pics").is_err());
    }
}
