use crate::app::App;
use crate::config::Side;
use crate::history::keep_visible;
use crate::preview::{Kind, Preview};
use crate::util::{format_time, w, wide};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

// COLORREF is 0x00BBGGRR
const BG: u32 = 0x0020_2020;
const FG: u32 = 0x00F0_F0F0;
const FG2: u32 = 0x00A8_A8A8;
const ACCENT: u32 = 0x00D7_7800;
const SEL: u32 = 0x0070_4010;
const LINE: u32 = 0x0040_4040;

/// History rows visible at once; longer histories scroll (no scrollbar).
pub const LIST_ROWS: usize = 10;

/// Fonts for one DPI, recreated when the DPI changes.
pub struct Fonts {
    dpi: u32,
    title: HFONT,
    body: HFONT,
    small: HFONT,
}

impl Fonts {
    pub fn new(dpi: u32) -> Fonts {
        let f = |pt: i32, weight: i32| unsafe {
            CreateFontW(-(pt * dpi as i32 / 72), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr())
        };
        Fonts { dpi, title: f(12, 600), body: f(10, 400), small: f(9, 400) }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.title);
            DeleteObject(self.body);
            DeleteObject(self.small);
        }
    }
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

unsafe fn fill(dc: HDC, r: RECT, c: u32) {
    let b = CreateSolidBrush(c);
    FillRect(dc, &r, b);
    DeleteObject(b);
}

unsafe fn text(dc: HDC, font: HFONT, s: &[u16], mut r: RECT, flags: u32, color: u32) {
    SelectObject(dc, font);
    SetTextColor(dc, color);
    DrawTextW(dc, s.as_ptr(), s.len() as i32, &mut r, flags | DT_NOPREFIX);
}

const LINE1: u32 = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;

pub fn paint(app: &mut App, hdc: HDC) {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(app.hwnd, &mut rc);
        let (wd, ht) = (rc.right, rc.bottom);
        if wd <= 0 || ht <= 0 {
            return;
        }
        let dpi = GetDpiForWindow(app.hwnd).max(96);
        if app.fonts.dpi != dpi {
            app.fonts = Fonts::new(dpi);
        }
        let s = |v: i32| v * dpi as i32 / 96;
        let dc = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, wd, ht);
        let old = SelectObject(dc, bmp);
        SetBkMode(dc, TRANSPARENT as _);
        fill(dc, rc, BG);
        // Accent bar on the screen-inner edge.
        let left = app.side() == Side::Left;
        let bar = if left { rect(wd - s(3), 0, wd, ht) } else { rect(0, 0, s(3), ht) };
        fill(dc, bar, ACCENT);

        let pad = s(14);
        let (x0, x1) = if left { (pad, wd - s(3) - pad) } else { (s(3) + pad, wd - pad) };
        let f = &app.fonts;
        let n = app.items.len();
        let mut y = pad;
        text(dc, f.title, &w("Clipboard"), rect(x0, y, x1, y + s(24)), LINE1 | DT_LEFT, FG);
        if n == 0 {
            text(dc, f.body, &w("History is empty"), rect(x0, y + s(40), x1, y + s(64)), LINE1 | DT_LEFT, FG2);
        } else {
            let sel = app.sel.min(n - 1);
            text(dc, f.title, &w(&format!("{} / {}", sel + 1, n)), rect(x0, y, x1, y + s(24)), LINE1 | DT_RIGHT, FG);
            let it = &app.items[sel];
            y += s(28);
            text(dc, f.small, &w(&it.preview.info), rect(x0, y, x1, y + s(18)), LINE1 | DT_LEFT, FG2);
            y += s(18);
            let when = format_time(it.time);
            let meta = if it.source.is_empty() { when } else { format!("{} · {}", when, it.source) };
            text(dc, f.small, &w(&meta), rect(x0, y, x1, y + s(18)), LINE1 | DT_LEFT, FG2);
            y += s(26);
            fill(dc, rect(x0, y, x1, y + 1), LINE);
            y += s(12);

            let row = s(24);
            let rows = n.min(LIST_ROWS);
            let first = keep_visible(sel, app.scroll, LIST_ROWS).min(n - rows);
            let list_top = ht - pad - row * rows as i32;
            preview(dc, f, &it.preview, rect(x0, y, x1, list_top - s(16)), &s);
            fill(dc, rect(x0, list_top - s(8), x1, list_top - s(8) + 1), LINE);
            for (i, item) in app.items.iter().enumerate().skip(first).take(rows) {
                let top = list_top + row * (i - first) as i32;
                if i == sel {
                    fill(dc, rect(x0 - s(6), top, x1 + s(6), top + row), SEL);
                }
                text(dc, f.small, &w(&(i + 1).to_string()), rect(x0, top, x0 + s(20), top + row), LINE1 | DT_LEFT, FG2);
                text(dc, f.body, &item.preview.line, rect(x0 + s(22), top, x1, top + row), LINE1 | DT_LEFT, if i == sel { FG } else { FG2 });
            }
        }
        BitBlt(hdc, 0, 0, wd, ht, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
    }
}

unsafe fn preview(dc: HDC, f: &Fonts, p: &Preview, r: RECT, s: &dyn Fn(i32) -> i32) {
    if r.bottom <= r.top {
        return;
    }
    let (rw, rh) = (r.right - r.left, r.bottom - r.top);
    match p.kind {
        Kind::Image => match &p.thumb {
            Some(t) => {
                let k = (rw as f64 / t.w as f64).min(rh as f64 / t.h as f64).min(1.0);
                let (dw, dh) = (((t.w as f64 * k) as i32).max(1), ((t.h as f64 * k) as i32).max(1));
                let src = CreateCompatibleDC(dc);
                let old = SelectObject(src, t.hbmp);
                SetStretchBltMode(dc, HALFTONE);
                SetBrushOrgEx(dc, 0, 0, null_mut());
                StretchBlt(dc, r.left + (rw - dw) / 2, r.top, dw, dh, src, 0, 0, t.w, t.h, SRCCOPY);
                SelectObject(src, old);
                DeleteDC(src);
            }
            None => text(dc, f.body, &w("(no preview)"), r, LINE1 | DT_LEFT, FG2),
        },
        Kind::Files => {
            let rh1 = s(20);
            let mut y = r.top;
            if !p.text.is_empty() {
                text(dc, f.small, &p.text, RECT { bottom: y + rh1, ..r }, LINE1 | DT_LEFT | DT_PATH_ELLIPSIS, FG2);
                y += rh1 + s(6);
            }
            let size_w = s(72);
            for (i, (name, size)) in p.files.iter().enumerate() {
                if y + rh1 * 2 > r.bottom && i + 1 < p.files.len() {
                    let more = w(&format!("+{} more", p.files.len() - i));
                    text(dc, f.small, &more, RECT { top: y, bottom: y + rh1, ..r }, LINE1 | DT_LEFT, FG2);
                    break;
                }
                text(dc, f.body, name, rect(r.left, y, r.right - size_w, y + rh1), LINE1 | DT_LEFT, FG);
                text(dc, f.small, size, rect(r.right - size_w, y, r.right, y + rh1), LINE1 | DT_RIGHT, FG2);
                y += rh1;
            }
        }
        _ => text(dc, f.body, &p.text, r, DT_LEFT | DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL, FG),
    }
}
