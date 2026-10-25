//! Low-level keyboard hook on its own thread.
//!
//! The state machine (`step`) is pure; the hook callback applies its output:
//! swallow the key, post actions to the main window, start/stop the hold timer,
//! and inject "mask key + Win up" so the Start menu does not open.

use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_HOOK: u32 = WM_APP + 1;
pub const A_SHOW: usize = 1;
pub const A_PREV: usize = 2;
pub const A_NEXT: usize = 3;
pub const A_COMMIT: usize = 4;
pub const A_CANCEL: usize = 5;
pub const A_DELETE: usize = 6;
pub const A_SAVE: usize = 7;

/// dwExtraInfo of our injected keys, so the hook ignores them.
const MARK: usize = 0x434C_4950;
/// Unassigned virtual key, sent before Win up to stop the Start menu.
const VK_MASK: u16 = 0xE8;

const VK_LWIN_: u32 = 0x5B;
const VK_RWIN_: u32 = 0x5C;
const VK_C: u32 = 0x43;
const VK_V: u32 = 0x56;
const VK_B: u32 = 0x42;
const VK_S: u32 = 0x53;
const VK_ESC: u32 = 0x1B;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Phase {
    Idle,
    /// Win is down, waiting for hold delay or C/V.
    Armed,
    /// Overlay is visible.
    Shown,
    /// Overlay closed with Esc; Win still down.
    Cancelled,
    /// Win used for a normal Windows shortcut.
    Passthrough,
}

pub struct St {
    pub phase: Phase,
    pub win: u32,
    /// Held modifiers: 1 Shift, 2 Ctrl, 4 Alt.
    pub mods: u8,
    /// The current clipboard item is an image (Win+S is only taken then).
    pub image: bool,
}

impl St {
    pub const fn new() -> St {
        St { phase: Phase::Idle, win: 0, mods: 0, image: false }
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Out {
    pub swallow: bool,
    /// Inject mask key + Win up (the real Win up is swallowed).
    pub inject: bool,
    pub posts: Vec<usize>,
    pub start_timer: bool,
    pub stop_timer: bool,
}

fn is_win(vk: u32) -> bool {
    vk == VK_LWIN_ || vk == VK_RWIN_
}

fn is_modifier(vk: u32) -> bool {
    modifier_bit(vk) != 0
}

fn modifier_bit(vk: u32) -> u8 {
    match vk {
        0x10 | 0xA0 | 0xA1 => 1,
        0x11 | 0xA2 | 0xA3 => 2,
        0x12 | 0xA4 | 0xA5 => 4,
        _ => 0,
    }
}

/// One key event → what to do.
pub fn step(s: &mut St, vk: u32, down: bool) -> Out {
    let mut o = Out::default();
    let bit = modifier_bit(vk);
    if down {
        s.mods |= bit;
    } else {
        s.mods &= !bit;
    }
    if s.phase == Phase::Idle {
        if down && is_win(vk) {
            s.phase = Phase::Armed;
            s.win = vk;
            o.start_timer = true;
        }
        return o;
    }
    if vk == s.win {
        if down {
            return o; // auto-repeat
        }
        o.stop_timer = true;
        match std::mem::replace(&mut s.phase, Phase::Idle) {
            Phase::Shown => {
                o.posts.push(A_COMMIT);
                o.swallow = true;
                o.inject = true;
            }
            Phase::Cancelled => {
                o.swallow = true;
                o.inject = true;
            }
            _ => {}
        }
        return o;
    }
    if is_win(vk) || is_modifier(vk) || s.phase == Phase::Passthrough {
        return o;
    }
    // Win+S only when an image is on the clipboard and no modifier is held,
    // so Windows Search and Win+Shift+S (snipping) keep working otherwise.
    let save = vk == VK_S && s.image && s.mods == 0;
    if vk == VK_C || vk == VK_V || vk == VK_B || save {
        o.swallow = true;
        if down {
            if s.phase != Phase::Shown {
                o.stop_timer = true;
                s.phase = Phase::Shown;
                o.posts.push(A_SHOW);
            }
            o.posts.push(match vk {
                VK_V => A_PREV,
                VK_C => A_NEXT,
                VK_B => A_DELETE,
                _ => A_SAVE,
            });
        }
        return o;
    }
    if vk == VK_ESC && matches!(s.phase, Phase::Shown | Phase::Cancelled) {
        o.swallow = true;
        if down && s.phase == Phase::Shown {
            s.phase = Phase::Cancelled;
            o.posts.push(A_CANCEL);
        }
        return o;
    }
    if down {
        if s.phase == Phase::Shown {
            o.posts.push(A_CANCEL);
        }
        o.stop_timer = true;
        s.phase = Phase::Passthrough;
    }
    o
}

/// Hold delay elapsed.
pub fn on_hold(s: &mut St) -> Option<usize> {
    if s.phase == Phase::Armed {
        s.phase = Phase::Shown;
        return Some(A_SHOW);
    }
    None
}

static MAIN: AtomicIsize = AtomicIsize::new(0);
static IMAGE: AtomicBool = AtomicBool::new(false);

/// Called by the main thread whenever the current clipboard item changes.
pub fn set_image_on_clipboard(on: bool) {
    IMAGE.store(on, Ordering::Relaxed);
}
static HOLD_MS: AtomicU32 = AtomicU32::new(250);

thread_local! {
    static ST: RefCell<St> = const { RefCell::new(St::new()) };
    static TIMER: RefCell<usize> = const { RefCell::new(0) };
}

/// Start the hook thread. Actions are posted to `main` as WM_APP_HOOK(wParam = action).
pub fn start(main: HWND, hold_ms: u32) {
    MAIN.store(main as isize, Ordering::Relaxed);
    HOLD_MS.store(hold_ms, Ordering::Relaxed);
    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), GetModuleHandleW(std::ptr::null()), 0);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER && msg.hwnd.is_null() {
                timer_fired(msg.wParam);
            }
        }
        UnhookWindowsHookEx(hook);
    });
}

fn post(a: usize) {
    unsafe { PostMessageW(MAIN.load(Ordering::Relaxed) as HWND, WM_APP_HOOK, a, 0) };
}

fn set_timer(on: bool) {
    TIMER.with(|t| unsafe {
        let mut t = t.borrow_mut();
        if *t != 0 {
            KillTimer(null_mut(), *t);
            *t = 0;
        }
        if on {
            *t = SetTimer(null_mut(), 0, HOLD_MS.load(Ordering::Relaxed), None);
        }
    });
}

fn timer_fired(id: usize) {
    let mine = TIMER.with(|t| *t.borrow() == id);
    if !mine {
        return;
    }
    set_timer(false);
    if let Some(a) = ST.with(|s| on_hold(&mut s.borrow_mut())) {
        post(a);
    }
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp as *const KBDLLHOOKSTRUCT);
        if k.dwExtraInfo != MARK {
            let m = wp as u32;
            let down = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
            let (o, win) = ST.with(|s| {
                let mut s = s.borrow_mut();
                s.image = IMAGE.load(Ordering::Relaxed);
                let win = s.win;
                (step(&mut s, k.vkCode, down), win)
            });
            if o.stop_timer {
                set_timer(false);
            }
            if o.start_timer {
                set_timer(true);
            }
            for a in &o.posts {
                post(*a);
            }
            if o.inject {
                inject_release(win as u16);
            }
            if o.swallow {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

unsafe fn inject_release(win: u16) {
    let key = |vk: u16, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MARK } },
    };
    let inputs = [key(VK_MASK, 0), key(VK_MASK, KEYEVENTF_KEYUP), key(win, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY)];
    SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> St {
        St::new()
    }

    #[test]
    fn s_saves_only_image_without_modifiers() {
        // no image: Win+S passes (Windows Search)
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, VK_S, true);
        assert!(!o.swallow && o.posts.is_empty());
        assert_eq!(s.phase, Phase::Passthrough);
        step(&mut s, VK_LWIN_, false);
        // image: Win+S is ours
        s.image = true;
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, VK_S, true);
        assert_eq!(o.posts, vec![A_SHOW, A_SAVE]);
        assert!(o.swallow);
        assert!(step(&mut s, VK_S, false).swallow);
        assert_eq!(step(&mut s, VK_LWIN_, false).posts, vec![A_COMMIT]);
        // image + Shift: Win+Shift+S passes (snipping tool)
        step(&mut s, 0xA0, true);
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, VK_S, true);
        assert!(!o.swallow && o.posts.is_empty());
        step(&mut s, VK_S, false);
        step(&mut s, VK_LWIN_, false);
        step(&mut s, 0xA0, false);
        assert_eq!(s.mods, 0);
    }

    #[test]
    fn quick_tap_opens_start() {
        let mut s = st();
        let o = step(&mut s, VK_LWIN_, true);
        assert!(o.start_timer && !o.swallow);
        let o = step(&mut s, VK_LWIN_, false);
        assert!(!o.swallow && !o.inject && o.posts.is_empty());
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn hold_shows_and_release_commits() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        assert_eq!(on_hold(&mut s), Some(A_SHOW));
        let o = step(&mut s, VK_LWIN_, true); // auto-repeat
        assert_eq!(o, Out::default());
        let o = step(&mut s, VK_LWIN_, false);
        assert_eq!(o.posts, vec![A_COMMIT]);
        assert!(o.swallow && o.inject);
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn v_then_c_navigates_and_swallows() {
        let mut s = st();
        step(&mut s, VK_RWIN_, true);
        let o = step(&mut s, VK_V, true);
        assert_eq!(o.posts, vec![A_SHOW, A_PREV]);
        assert!(o.swallow && o.stop_timer);
        assert!(step(&mut s, VK_V, false).swallow);
        let o = step(&mut s, VK_V, true);
        assert_eq!(o.posts, vec![A_PREV]);
        let o = step(&mut s, VK_C, true);
        assert_eq!(o.posts, vec![A_NEXT]);
        assert!(on_hold(&mut s).is_none());
        let o = step(&mut s, VK_RWIN_, false);
        assert_eq!(o.posts, vec![A_COMMIT]);
        assert!(o.inject);
    }

    #[test]
    fn b_deletes_and_swallows() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, VK_B, true);
        assert_eq!(o.posts, vec![A_SHOW, A_DELETE]);
        assert!(o.swallow);
        assert!(step(&mut s, VK_B, false).swallow);
        assert_eq!(step(&mut s, VK_B, true).posts, vec![A_DELETE]);
        let o = step(&mut s, VK_LWIN_, false);
        assert_eq!(o.posts, vec![A_COMMIT]);
        assert!(o.swallow && o.inject); // Start menu stays closed
    }

    #[test]
    fn other_shortcut_passes_through() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, 0x45, true); // Win+E
        assert!(!o.swallow && o.stop_timer && o.posts.is_empty());
        assert_eq!(s.phase, Phase::Passthrough);
        assert!(!step(&mut s, VK_V, true).swallow); // later V is not ours
        assert!(on_hold(&mut s).is_none());
        let o = step(&mut s, VK_LWIN_, false);
        assert!(!o.swallow && !o.inject);
    }

    #[test]
    fn other_key_while_shown_cancels() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        on_hold(&mut s);
        let o = step(&mut s, 0x44, true); // Win+D
        assert_eq!(o.posts, vec![A_CANCEL]);
        assert!(!o.swallow);
        assert!(!step(&mut s, VK_LWIN_, false).inject);
    }

    #[test]
    fn modifiers_do_not_change_state() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, 0xA0, true); // LShift
        assert!(!o.swallow);
        assert_eq!(s.phase, Phase::Armed);
    }

    #[test]
    fn escape_cancels_and_masks_start() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        on_hold(&mut s);
        let o = step(&mut s, VK_ESC, true);
        assert_eq!(o.posts, vec![A_CANCEL]);
        assert!(o.swallow);
        assert!(step(&mut s, VK_ESC, false).swallow);
        let o = step(&mut s, VK_LWIN_, false);
        assert!(o.swallow && o.inject && o.posts.is_empty());
    }

    #[test]
    fn escape_without_overlay_passes() {
        let mut s = st();
        step(&mut s, VK_LWIN_, true);
        let o = step(&mut s, VK_ESC, true);
        assert!(!o.swallow);
        assert_eq!(s.phase, Phase::Passthrough);
    }

    #[test]
    fn keys_without_win_ignored() {
        let mut s = st();
        assert_eq!(step(&mut s, VK_V, true), Out::default());
        assert_eq!(s.phase, Phase::Idle);
    }
}
