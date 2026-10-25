use crate::clip::{self, Payload};
use crate::config::{self, Config, Side};
use crate::drag;
use crate::history::{self, Item, RAM_LIMIT};
use crate::hook::{self, A_CANCEL, A_COMMIT, A_DELETE, A_NEXT, A_PREV, A_SAVE, A_SHOW, A_TAP, WM_APP_HOOK};
use crate::imagesave;
use crate::paint::{self, Fonts};
use crate::preview::{self, Kind};
use crate::settings;
use crate::store::{self, Entry, Store};
use crate::tray;
use crate::util::{data_dir, now_unix, wide};
use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{DragDetect, GetAsyncKeyState, VK_LWIN, VK_RWIN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_SAVED: u32 = WM_APP + 2;
/// Sent by a second instance: open the settings window.
pub const WM_APP_SETTINGS: u32 = WM_APP + 4;
/// The foreground window changed (lParam = new foreground HWND).
const WM_APP_FOREGROUND: u32 = WM_APP + 5;
const TIMER_CAPTURE: usize = 1;
const TIMER_WATCH: usize = 2;
/// After a quick Win tap: wait this long for the Start menu to open (ShowWithStart mode).
const TIMER_TAP: usize = 3;
const TAP_WAIT_MS: u32 = 600;
/// Processes that own the open Start menu: StartMenuExperienceHost on Windows 10 and
/// older Windows 11, SearchHost on newer Windows 11 (it also hosts the Search panel).
const START_MENU_EXES: [&str; 2] = ["StartMenuExperienceHost.exe", "SearchHost.exe"];

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub struct App {
    pub hwnd: HWND,
    pub items: Vec<Item>,
    /// Index of the item that is on the clipboard now (the overlay opens on it).
    cur: usize,
    pub sel: usize,
    /// First history row shown in the list (it shows at most `paint::LIST_ROWS` rows).
    pub scroll: usize,
    /// Horizontal content offset during the slide-in (0 when settled).
    pub slide: i32,
    /// Window rect (x, y, width, height) of the overlay on the current monitor.
    pub frame: (i32, i32, i32, i32),
    /// DPI of that monitor.
    pub dpi: u32,
    pub fonts: Fonts,
    /// File panel of the shown item, in window coordinates (empty if it is not a file item).
    pub drag_area: RECT,
    /// File rows drawn in the file panel: (top, bottom, file index).
    pub drag_rows: Vec<(i32, i32, usize)>,
    cfg: Config,
    fmts: clip::Formats,
    visible: bool,
    ignore_seq: u32,
    store: Store,
    dir: PathBuf,
    thumb_w: i32,
    /// The Start menu is the foreground window.
    start_open: bool,
    /// A quick Win tap happened; waiting up to TAP_WAIT_MS for Start to open.
    tap_pending: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Max thumbnail width: the overlay width on the primary monitor minus padding.
fn thumb_width(width_pct: u32) -> i32 {
    unsafe { GetSystemMetrics(SM_CXSCREEN) * width_pct as i32 / 100 - 32 }
}

/// Called by the settings window on OK.
pub fn apply_settings(cfg: Config) {
    with_app(|a| a.apply_settings(cfg));
}

/// Run `f` on the app state; None if it is busy (re-entrant message).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn run(cfg: Config) {
    unsafe {
        drag::init();
        let inst = GetModuleHandleW(null());
        let class = wide("UnshufflzOverlay");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
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
        // Layered window drawn with UpdateLayeredWindow (per-pixel alpha), see paint::render.
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

        let dir = data_dir().join("store");
        let thumb_w = thumb_width(cfg.width_pct);
        let mut app = App {
            hwnd,
            items: Vec::new(),
            cur: 0,
            sel: 0,
            scroll: 0,
            slide: 0,
            frame: (0, 0, 0, 0),
            dpi: 96,
            fonts: Fonts::new(96),
            drag_area: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            drag_rows: Vec::new(),
            fmts: clip::Formats::register(),
            visible: false,
            ignore_seq: 0,
            store: Store::start(dir.clone(), hwnd as isize, WM_APP_SAVED),
            dir,
            thumb_w,
            cfg,
            start_open: is_start_menu(GetForegroundWindow()),
            tap_pending: false,
        };
        app.load();
        app.update_image_flag();
        app.on_clipboard(true); // take what is on the clipboard now
        let hold = app.cfg.hold_ms;
        APP.with(|a| *a.borrow_mut() = Some(app));

        AddClipboardFormatListener(hwnd);
        tray::add(hwnd);
        hook::start(hwnd, hold);
        // Foreground changes tell us when the Start menu opens or closes.
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, null_mut(), Some(on_win_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let sh = settings::hwnd();
            if !sh.is_null() && IsDialogMessageW(sh, &msg) != 0 {
                continue; // Tab / Enter / Esc in the settings window
            }
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
        for (e, p) in store::load(&self.dir, self.cfg.max_items) {
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

    /// `startup`: the clipboard may hold an older history item (chosen before the
    /// last exit); then that item becomes current and the order is kept.
    fn on_clipboard(&mut self, startup: bool) {
        if unsafe { GetClipboardSequenceNumber() } == self.ignore_seq {
            return; // our own restore
        }
        if let Some((p, src)) = clip::capture(self.hwnd, &self.fmts, self.cfg.max_item_bytes) {
            self.add(p, src, startup);
        }
    }

    fn add(&mut self, p: Payload, source: String, startup: bool) {
        let id = history::full_hash(&p);
        if self.items.get(self.cur).is_some_and(|it| it.id == id) {
            return;
        }
        if startup {
            if let Some(i) = self.items.iter().position(|it| it.id == id) {
                self.cur = i;
                self.update_image_flag();
                return;
            }
        }
        let key = history::content_key(&p);
        self.items.retain(|it| it.key != key);
        let item = self.new_item(id, key, p, source);
        self.items.insert(0, item);
        self.items.truncate(self.cfg.max_items);
        self.cur = 0;
        self.sel = 0;
        self.save_index();
        self.invalidate();
    }

    /// Build the preview and send the payload to the writer.
    fn new_item(&mut self, id: u64, key: u64, p: Payload, source: String) -> Item {
        let preview = preview::build(&p, self.thumb_w);
        let size = history::payload_size(&p);
        let p = Arc::new(p);
        self.store.save_item(id, p.clone());
        Item { id, key, time: now_unix(), source, size, payload: Some(p), preview }
    }

    /// Called after every history change: persist the order and tell the hook
    /// whether Win+S applies (current item is an image).
    fn save_index(&self) {
        let e = self.items.iter().map(|it| Entry { id: it.id, time: it.time, source: it.source.clone() }).collect();
        self.store.save_index(e);
        self.update_image_flag();
    }

    fn update_image_flag(&self) {
        hook::set_image_on_clipboard(self.items.get(self.cur).is_some_and(|it| it.preview.kind == Kind::Image));
    }

    fn on_saved(&mut self, id: u64) {
        if let Some(it) = self.items.iter_mut().find(|it| it.id == id) {
            if it.size > RAM_LIMIT {
                it.payload = None;
            }
        }
    }

    fn on_hook(&mut self, a: usize) {
        crate::util::debug(|| format!("main action {} visible={}", a, self.visible));
        match a {
            // ShowWithStart: Win pressed while Start is open closes Start; don't show then.
            // (C/V/B/S during that press still show the overlay.)
            A_SHOW if self.cfg.show_with_start && self.start_open && !self.visible => {}
            A_SHOW => self.show(),
            A_PREV => {
                self.show();
                if self.sel + 1 < self.items.len() {
                    self.sel += 1;
                }
                self.scroll = history::keep_visible(self.sel, self.scroll, paint::LIST_ROWS);
                self.invalidate();
            }
            A_NEXT => {
                self.show();
                self.sel = self.sel.saturating_sub(1);
                self.scroll = history::keep_visible(self.sel, self.scroll, paint::LIST_ROWS);
                self.invalidate();
            }
            A_COMMIT => {
                self.hide();
                if self.sel != self.cur && self.sel < self.items.len() {
                    self.select(self.sel);
                }
            }
            A_CANCEL => {
                self.hide();
                self.sel = 0;
            }
            A_TAP if self.cfg.show_with_start && !self.start_open && self.visible => {
                // Start is opening: keep the overlay; on_foreground() takes over, or the
                // timer hides it if Start did not open.
                self.tap_pending = true;
                unsafe { SetTimer(self.hwnd, TIMER_TAP, TAP_WAIT_MS, None) };
            }
            A_TAP => {
                self.hide();
                self.sel = 0;
            }
            A_SAVE => {
                self.show();
                self.save_image();
                self.sel = self.cur;
                self.scroll = history::keep_visible(self.sel, self.scroll, paint::LIST_ROWS);
                self.invalidate();
            }
            A_DELETE => {
                self.show();
                self.delete(self.sel);
                self.scroll = history::keep_visible(self.sel, self.scroll, paint::LIST_ROWS);
                self.invalidate();
            }
            _ => {}
        }
    }

    /// Payload of item `i` from RAM or disk. If it can't be read: beep and remove the item.
    fn payload(&mut self, i: usize) -> Option<Arc<Payload>> {
        if let Some(p) = &self.items[i].payload {
            return Some(p.clone());
        }
        match store::read_item(&self.dir, self.items[i].id) {
            Some(p) => Some(Arc::new(p)),
            None => {
                unsafe { MessageBeep(MB_ICONWARNING) };
                self.items.remove(i);
                if i < self.cur {
                    self.cur -= 1;
                }
                self.cur = self.cur.min(self.items.len().saturating_sub(1));
                self.save_index();
                None
            }
        }
    }

    /// Put item `i` on the clipboard and make it current. The list order is kept, so
    /// "Ctrl+V, Win+V" walks down the history one item at a time.
    fn select(&mut self, i: usize) {
        let Some(payload) = self.payload(i) else { return };
        if clip::restore(self.hwnd, &payload) {
            self.ignore_seq = unsafe { GetClipboardSequenceNumber() };
        }
        self.cur = i;
        self.update_image_flag();
    }

    /// Win+B: delete item `i` from history (and disk). If it was the clipboard content
    /// (i == cur), the next older item goes on the clipboard (the newer one if it was the
    /// oldest), or the clipboard is emptied.
    fn delete(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        self.items.remove(i);
        if i < self.cur {
            self.cur -= 1;
        } else if i == self.cur {
            loop {
                self.cur = self.cur.min(self.items.len().saturating_sub(1));
                if self.items.is_empty() {
                    if clip::clear(self.hwnd) {
                        self.ignore_seq = unsafe { GetClipboardSequenceNumber() };
                    }
                    break;
                }
                // payload() drops unreadable items, so try the next one then.
                if let Some(p) = self.payload(self.cur) {
                    if clip::restore(self.hwnd, &p) {
                        self.ignore_seq = unsafe { GetClipboardSequenceNumber() };
                    }
                    break;
                }
            }
        }
        self.sel = self.sel.min(self.items.len().saturating_sub(1));
        self.save_index();
    }

    /// Win+S: save the current (image) item to Pictures\Clipboard as PNG, replace it in
    /// history (same position) with the saved file, and put that file on the clipboard
    /// (as Explorer's Copy).
    fn save_image(&mut self) {
        let i = self.cur;
        if self.items.get(i).is_none_or(|it| it.preview.kind != Kind::Image) {
            return;
        }
        let Some(p) = self.payload(i) else { return };
        let path = match imagesave::save_png(&p, &imagesave::target_dir(&self.cfg.pictures_dir)) {
            Ok(path) => path,
            Err(_) => {
                unsafe { MessageBeep(MB_ICONWARNING) };
                return;
            }
        };
        let fp = imagesave::file_payload(&path);
        let (id, key) = (history::full_hash(&fp), history::content_key(&fp));
        let item = self.new_item(id, key, fp, "unshufflz.exe".into());
        let p = item.payload.clone();
        self.items[i] = item;
        self.save_index();
        if let Some(p) = p {
            if clip::restore(self.hwnd, &p) {
                self.ignore_seq = unsafe { GetClipboardSequenceNumber() };
            }
        }
    }

    fn show(&mut self) {
        if self.visible {
            return;
        }
        self.sel = self.cur.min(self.items.len().saturating_sub(1));
        self.scroll = history::keep_visible(self.sel, 0, paint::LIST_ROWS);
        unsafe {
            let mut pt = std::mem::zeroed();
            GetCursorPos(&mut pt);
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let w = ((mi.rcMonitor.right - mi.rcMonitor.left) * self.cfg.width_pct as i32 / 100).max(200);
            let wa = mi.rcWork;
            let x = if self.cfg.side == Side::Left { wa.left } else { wa.right - w };
            let h = wa.bottom - wa.top;
            let (mut dx, mut dy) = (96, 96);
            GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            self.dpi = dx;
            self.frame = (x, wa.top, w, h);
            let animate = self.cfg.slide_ms() > 0;
            // First frame: fully at the screen edge (nothing visible) when animating.
            self.slide = if !animate { 0 } else if self.cfg.side == Side::Left { -w } else { w };
            paint::render(self);
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetTimer(self.hwnd, TIMER_WATCH, 150, None);
            self.visible = true;
            self.update_click_through();
            crate::util::debug(|| "main overlay shown".to_string());
            if animate {
                self.slide_in(w);
            }
        }
    }

    /// Slide the content in from the screen edge (blocks the main thread for `slide_ms()`;
    /// keys keep queueing in the hook thread). One frame per screen refresh (DwmFlush).
    fn slide_in(&mut self, w: i32) {
        let sign = if self.cfg.side == Side::Left { -1 } else { 1 };
        let ms = self.cfg.slide_ms() as f64;
        let start = std::time::Instant::now();
        let mut frames = 0;
        loop {
            let dx = crate::anim::slide_offset(w, start.elapsed().as_secs_f64() * 1000.0, ms);
            self.slide = sign * dx;
            paint::render(self);
            if dx == 0 {
                crate::util::debug(|| format!("main slide-in done: {} frames in {:.1} ms", frames, start.elapsed().as_secs_f64() * 1000.0));
                break;
            }
            frames += 1;
            // One frame per screen refresh.
            if unsafe { DwmFlush() } < 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
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
    /// In ShowWithStart mode the overlay stays while Start is open (or about to open).
    fn watch(&mut self) {
        if self.cfg.show_with_start && (self.start_open || self.tap_pending) {
            return;
        }
        if !win_down() {
            self.hide();
            self.sel = 0;
        }
    }

    fn on_tap_timer(&mut self) {
        unsafe { KillTimer(self.hwnd, TIMER_TAP) };
        self.tap_pending = false;
        if !self.start_open {
            self.hide();
            self.sel = 0;
        }
    }

    /// The foreground window changed: track whether the Start menu is open.
    fn on_foreground(&mut self, fg: HWND) {
        let open = is_start_menu(fg);
        if open == self.start_open {
            return;
        }
        self.start_open = open;
        crate::util::debug(|| format!("main start menu open={}", open));
        if !self.cfg.show_with_start {
            return;
        }
        if open {
            self.tap_pending = false;
            unsafe { KillTimer(self.hwnd, TIMER_TAP) };
            self.show();
        } else if !win_down() {
            // Start closed (Esc, click elsewhere, app launched). If Win is held, the
            // key release decides instead.
            self.hide();
            self.sel = 0;
        }
    }

    /// Redraw now (the overlay is a layered window; there is no WM_PAINT drawing).
    fn invalidate(&mut self) {
        if self.visible {
            paint::render(self);
            self.update_click_through();
        }
    }

    /// The overlay lets the mouse through, except while a file item is shown:
    /// then its file panel can be dragged (see `drag_paths`).
    fn update_click_through(&self) {
        unsafe {
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
            let want = if self.has_drag_area() { ex & !WS_EX_TRANSPARENT } else { ex | WS_EX_TRANSPARENT };
            if want != ex {
                SetWindowLongW(self.hwnd, GWL_EXSTYLE, want as i32);
            }
        }
    }

    fn has_drag_area(&self) -> bool {
        let r = self.drag_area;
        r.right > r.left && r.bottom > r.top
    }

    fn in_drag_area(&self, x: i32, y: i32) -> bool {
        let r = self.drag_area;
        self.visible && x >= r.left && x < r.right && y >= r.top && y < r.bottom
    }

    /// Files to drag for a mouse press at (x, y) (window coordinates): the file under
    /// the mouse, or all files of the shown item when pressed elsewhere in the panel.
    fn drag_paths(&mut self, x: i32, y: i32) -> Vec<String> {
        if !self.in_drag_area(x, y) || self.sel >= self.items.len() {
            return Vec::new();
        }
        let row = self.drag_rows.iter().find(|r| y >= r.0 && y < r.1).map(|r| r.2);
        let Some(p) = self.payload(self.sel) else { return Vec::new() };
        let Some(f) = p.iter().find(|f| f.id == clip::CF_HDROP) else { return Vec::new() };
        let mut paths = preview::parse_hdrop(&f.data);
        if let Some(i) = row.filter(|&i| i < paths.len()) {
            paths = vec![paths.swap_remove(i)];
        }
        paths
    }

    pub fn side(&self) -> Side {
        self.cfg.side
    }

    fn apply_settings(&mut self, cfg: Config) {
        let width_changed = cfg.width_pct != self.cfg.width_pct;
        let (width_pct, max_items) = (cfg.width_pct, cfg.max_items);
        hook::set_hold_ms(cfg.hold_ms);
        self.cfg = cfg;
        if self.items.len() > max_items {
            self.items.truncate(max_items);
            self.cur = self.cur.min(max_items - 1);
            self.save_index(); // the writer then deletes the dropped item files
        }
        let _ = config::save(&self.cfg);
        if width_changed {
            // Rebuild thumbnails (images, single image files) for the new width.
            self.thumb_w = thumb_width(width_pct);
            for it in &mut self.items {
                if it.preview.kind != Kind::Image && it.preview.thumb.is_none() {
                    continue;
                }
                let p = it.payload.clone().or_else(|| store::read_item(&self.dir, it.id).map(Arc::new));
                if let Some(p) = p {
                    it.preview = preview::build(&p, self.thumb_w);
                }
            }
        }
    }

    fn on_menu(&mut self, cmd: u32) {
        match cmd {
            tray::ID_SETTINGS => settings::open(self.cfg.clone()),
            tray::ID_CLEAR => {
                self.items.clear();
                self.cur = 0;
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

fn win_down() -> bool {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 };
    down(VK_LWIN) || down(VK_RWIN)
}

fn is_start_menu(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let exe = crate::util::window_exe_name(hwnd);
    START_MENU_EXES.iter().any(|s| exe.eq_ignore_ascii_case(s))
}

/// WinEvent callback (main thread). Posted on, so the app state is never borrowed twice.
unsafe extern "system" fn on_win_event(_: HWINEVENTHOOK, _event: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    let main = FindWindowW(wide("UnshufflzOverlay").as_ptr(), null());
    if !main.is_null() {
        PostMessageW(main, WM_APP_FOREGROUND, 0, hwnd as LPARAM);
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_APP_HOOK => {
            with_app(|a| a.on_hook(wp));
            0
        }
        WM_APP_SETTINGS => {
            with_app(|a| a.on_menu(tray::ID_SETTINGS));
            0
        }
        WM_APP_FOREGROUND => {
            with_app(|a| a.on_foreground(lp as HWND));
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
                    if with_app(|a| a.on_clipboard(false)).is_none() {
                        SetTimer(h, TIMER_CAPTURE, 80, None);
                    }
                }
                TIMER_WATCH => {
                    with_app(|a| a.watch());
                }
                TIMER_TAP => {
                    with_app(|a| a.on_tap_timer());
                }
                _ => {}
            }
            0
        }
        WM_PAINT => {
            // Drawing happens in paint::render (UpdateLayeredWindow); just validate.
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(h, &mut ps);
            EndPaint(h, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            let mut pt = std::mem::zeroed();
            GetCursorPos(&mut pt);
            ScreenToClient(h, &mut pt);
            let hand = with_app(|a| a.in_drag_area(pt.x, pt.y)).unwrap_or(false);
            SetCursor(LoadCursorW(null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
            1
        }
        WM_LBUTTONDOWN => {
            // Drag files out of the file panel. The drag runs outside the app borrow,
            // so hook messages (e.g. Win released) are still handled during it.
            let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            let paths = with_app(|a| a.drag_paths(x, y)).unwrap_or_default();
            let mut pt = POINT { x, y };
            ClientToScreen(h, &mut pt);
            if !paths.is_empty() && DragDetect(h, pt) != 0 {
                drag::drag_files(h, &paths);
            }
            0
        }
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
