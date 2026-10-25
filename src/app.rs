use crate::clip::{self, Payload};
use crate::config::Config;
use crate::history::{self, Item, MAX, RAM_LIMIT};
use crate::hook::{self, A_CANCEL, A_COMMIT, A_NEXT, A_PREV, A_SHOW, WM_APP_HOOK};
use crate::paint::{self, Fonts};
use crate::preview;
use crate::store::{self, Entry, Store};
use crate::tray;
use crate::util::{data_dir, now_unix, wide};
use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LWIN, VK_RWIN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_SAVED: u32 = WM_APP + 2;
const TIMER_CAPTURE: usize = 1;
const TIMER_WATCH: usize = 2;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub struct App {
    pub hwnd: HWND,
    pub items: Vec<Item>,
    pub sel: usize,
    pub fonts: Fonts,
    cfg: Config,
    fmts: clip::Formats,
    visible: bool,
    ignore_seq: u32,
    store: Store,
    dir: PathBuf,
    thumb_w: i32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` on the app state; None if it is busy (re-entrant message).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn run(cfg: Config) {
    unsafe {
        let inst = GetModuleHandleW(null());
        let class = wide("UnshufflzOverlay");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class.as_ptr(),
            wide("Unshufflz").as_ptr(),
            WS_POPUP,
            0, 0, 0, 0,
            null_mut(),
            null_mut(),
            inst,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        SetLayeredWindowAttributes(hwnd, 0, 245, LWA_ALPHA);
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

        let dir = data_dir().join("store");
        let thumb_w = GetSystemMetrics(SM_CXSCREEN) * cfg.width_pct as i32 / 100 - 32;
        let mut app = App {
            hwnd,
            items: Vec::new(),
            sel: 0,
            fonts: Fonts::new(96),
            fmts: clip::Formats::register(),
            visible: false,
            ignore_seq: 0,
            store: Store::start(dir.clone(), hwnd as isize, WM_APP_SAVED),
            dir,
            thumb_w,
            cfg,
        };
        app.load();
        app.on_clipboard(); // take what is on the clipboard now
        let hold = app.cfg.hold_ms;
        APP.with(|a| *a.borrow_mut() = Some(app));

        AddClipboardFormatListener(hwnd);
        tray::add(hwnd);
        hook::start(hwnd, hold);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(mut app) = APP.with(|a| a.borrow_mut().take()) {
            app.store.shutdown();
        }
    }
}

impl App {
    fn load(&mut self) {
        for (e, p) in store::load(&self.dir, MAX) {
            let size = history::payload_size(&p);
            let item = Item {
                id: e.id,
                key: history::content_key(&p),
                time: e.time,
                source: e.source,
                size,
                preview: preview::build(&p, self.thumb_w),
                payload: (size <= RAM_LIMIT).then(|| Arc::new(p)),
            };
            self.items.push(item);
        }
    }

    fn on_clipboard(&mut self) {
        if unsafe { GetClipboardSequenceNumber() } == self.ignore_seq {
            return; // our own restore
        }
        if let Some((p, src)) = clip::capture(self.hwnd, &self.fmts, self.cfg.max_item_bytes) {
            self.add(p, src);
        }
    }

    fn add(&mut self, p: Payload, source: String) {
        let id = history::full_hash(&p);
        if self.items.first().is_some_and(|it| it.id == id) {
            return;
        }
        let key = history::content_key(&p);
        self.items.retain(|it| it.key != key);
        let preview = preview::build(&p, self.thumb_w);
        let size = history::payload_size(&p);
        let p = Arc::new(p);
        self.store.save_item(id, p.clone());
        self.items.insert(0, Item { id, key, time: now_unix(), source, size, payload: Some(p), preview });
        self.items.truncate(MAX);
        self.sel = 0;
        self.save_index();
        self.invalidate();
    }

    fn save_index(&self) {
        let e = self.items.iter().map(|it| Entry { id: it.id, time: it.time, source: it.source.clone() }).collect();
        self.store.save_index(e);
    }

    fn on_saved(&mut self, id: u64) {
        if let Some(it) = self.items.iter_mut().find(|it| it.id == id) {
            if it.size > RAM_LIMIT {
                it.payload = None;
            }
        }
    }

    fn on_hook(&mut self, a: usize) {
        match a {
            A_SHOW => self.show(),
            A_PREV => {
                self.show();
                if self.sel + 1 < self.items.len() {
                    self.sel += 1;
                }
                self.invalidate();
            }
            A_NEXT => {
                self.show();
                self.sel = self.sel.saturating_sub(1);
                self.invalidate();
            }
            A_COMMIT => {
                self.hide();
                if self.sel > 0 && self.sel < self.items.len() {
                    self.select(self.sel);
                }
                self.sel = 0;
            }
            A_CANCEL => {
                self.hide();
                self.sel = 0;
            }
            _ => {}
        }
    }

    /// Put item `i` on the clipboard and move it to the top.
    fn select(&mut self, i: usize) {
        let payload = match &self.items[i].payload {
            Some(p) => p.clone(),
            None => match store::read_item(&self.dir, self.items[i].id) {
                Some(p) => Arc::new(p),
                None => {
                    unsafe { MessageBeep(MB_ICONWARNING) };
                    self.items.remove(i);
                    self.save_index();
                    return;
                }
            },
        };
        if clip::restore(self.hwnd, &payload) {
            self.ignore_seq = unsafe { GetClipboardSequenceNumber() };
        }
        let mut it = self.items.remove(i);
        if it.size > RAM_LIMIT {
            it.payload = None;
        }
        self.items.insert(0, it);
        self.save_index();
    }

    fn show(&mut self) {
        if self.visible {
            return;
        }
        self.sel = 0;
        unsafe {
            let mut pt = std::mem::zeroed();
            GetCursorPos(&mut pt);
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let w = ((mi.rcMonitor.right - mi.rcMonitor.left) * self.cfg.width_pct as i32 / 100).max(200);
            let wa = mi.rcWork;
            SetWindowPos(self.hwnd, HWND_TOPMOST, wa.right - w, wa.top, w, wa.bottom - wa.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetTimer(self.hwnd, TIMER_WATCH, 150, None);
        }
        self.visible = true;
        self.invalidate();
    }

    fn hide(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
            KillTimer(self.hwnd, TIMER_WATCH);
        }
        self.visible = false;
    }

    /// Safety net: hide if Win is no longer down (e.g. the hook missed the key up).
    fn watch(&mut self) {
        let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 };
        if !down(VK_LWIN) && !down(VK_RWIN) {
            self.hide();
            self.sel = 0;
        }
    }

    fn invalidate(&self) {
        if self.visible {
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }
    }

    fn on_menu(&mut self, cmd: u32) {
        match cmd {
            tray::ID_CLEAR => {
                self.items.clear();
                self.sel = 0;
                self.save_index();
            }
            tray::ID_AUTOSTART => tray::set_autostart(!tray::autostart_enabled()),
            tray::ID_EXIT => unsafe {
                DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_APP_HOOK => {
            with_app(|a| a.on_hook(wp));
            0
        }
        WM_APP_SAVED => {
            with_app(|a| a.on_saved(lp as u64));
            0
        }
        WM_CLIPBOARDUPDATE => {
            // Short delay so bursts of updates are read once.
            SetTimer(h, TIMER_CAPTURE, 80, None);
            0
        }
        WM_TIMER => {
            match wp {
                TIMER_CAPTURE => {
                    KillTimer(h, TIMER_CAPTURE);
                    if with_app(|a| a.on_clipboard()).is_none() {
                        SetTimer(h, TIMER_CAPTURE, 80, None);
                    }
                }
                TIMER_WATCH => {
                    with_app(|a| a.watch());
                }
                _ => {}
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(h, &mut ps);
            with_app(|a| paint::paint(a, hdc));
            EndPaint(h, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_DPICHANGED => 0,
        tray::WM_APP_TRAY => {
            let ev = (lp & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                let cmd = tray::menu(h);
                with_app(|a| a.on_menu(cmd));
            }
            0
        }
        WM_ENDSESSION => {
            if wp != 0 {
                with_app(|a| a.store.shutdown());
            }
            0
        }
        WM_DESTROY => {
            RemoveClipboardFormatListener(h);
            tray::remove(h);
            PostQuitMessage(0);
            0
        }
        _ if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            tray::add(h);
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}
