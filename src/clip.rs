use crate::util::wide;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{DeleteEnhMetaFile, GetEnhMetaFileBits, SetEnhMetaFileBits};
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub const CF_TEXT: u32 = 1;
pub const CF_BITMAP: u32 = 2;
pub const CF_METAFILEPICT: u32 = 3;
pub const CF_OEMTEXT: u32 = 7;
pub const CF_DIB: u32 = 8;
pub const CF_PALETTE: u32 = 9;
pub const CF_UNICODETEXT: u32 = 13;
pub const CF_ENHMETAFILE: u32 = 14;
pub const CF_HDROP: u32 = 15;
pub const CF_DIBV5: u32 = 17;

/// One clipboard format. `name` is set for registered formats (id >= 0xC000),
/// because their ids differ between sessions.
pub struct Fmt {
    pub id: u32,
    pub name: String,
    pub data: Vec<u8>,
}

pub type Payload = Vec<Fmt>;

/// Registered format ids used for privacy checks.
pub struct Formats {
    exclude: u32,
    can_include: u32,
    viewer_ignore: u32,
}

impl Formats {
    pub fn register() -> Self {
        let r = |s: &str| unsafe { RegisterClipboardFormatW(wide(s).as_ptr()) };
        Formats {
            exclude: r("ExcludeClipboardContentFromMonitorProcessing"),
            can_include: r("CanIncludeInClipboardHistory"),
            viewer_ignore: r("Clipboard Viewer Ignore"),
        }
    }
}

fn open(hwnd: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        unsafe { Sleep(15) };
    }
    false
}

/// Snapshot of the current clipboard plus the source exe name.
/// None if empty, private, too big or unavailable.
pub fn capture(hwnd: HWND, f: &Formats, max_bytes: u64) -> Option<(Payload, String)> {
    if !open(hwnd) {
        return None;
    }
    let r = unsafe { capture_inner(f, max_bytes) };
    unsafe { CloseClipboard() };
    r.map(|p| (p, owner_name()))
}

unsafe fn capture_inner(f: &Formats, max_bytes: u64) -> Option<Payload> {
    if IsClipboardFormatAvailable(f.exclude) != 0 || IsClipboardFormatAvailable(f.viewer_ignore) != 0 {
        return None;
    }
    if IsClipboardFormatAvailable(f.can_include) != 0 {
        if let Some(d) = read_global(GetClipboardData(f.can_include)) {
            if d.len() >= 4 && u32::from_le_bytes([d[0], d[1], d[2], d[3]]) == 0 {
                return None;
            }
        }
    }
    let mut ids = Vec::new();
    let mut id = 0;
    loop {
        id = EnumClipboardFormats(id);
        if id == 0 {
            break;
        }
        ids.push(id);
    }
    let has_uni = ids.contains(&CF_UNICODETEXT);
    let has_v5 = ids.contains(&CF_DIBV5);
    let mut out = Vec::new();
    let mut total = 0u64;
    for &id in &ids {
        if skip(id, has_uni, has_v5) {
            continue;
        }
        let name = if id >= 0xC000 { format_name(id) } else { String::new() };
        if id >= 0xC000 && (name.is_empty() || name == "DataObject" || name == "Ole Private Data") {
            continue;
        }
        let h = GetClipboardData(id);
        if h.is_null() {
            continue;
        }
        let size = if id == CF_ENHMETAFILE {
            GetEnhMetaFileBits(h, 0, null_mut()) as u64
        } else {
            GlobalSize(h) as u64
        };
        total += size;
        if total > max_bytes {
            return None;
        }
        let data = if id == CF_ENHMETAFILE { read_emf(h) } else { read_global(h) };
        if let Some(data) = data {
            out.push(Fmt { id, name, data });
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Formats that are GDI handles, owner-drawn, private, or synthesized by Windows.
fn skip(id: u32, has_uni: bool, has_v5: bool) -> bool {
    match id {
        CF_BITMAP | CF_METAFILEPICT | CF_PALETTE => true,
        CF_TEXT | CF_OEMTEXT => has_uni,
        CF_DIB => has_v5,
        0x0080..=0x008F => true, // CF_OWNERDISPLAY, CF_DSP*
        0x0200..=0x03FF => true, // CF_PRIVATEFIRST..CF_GDIOBJLAST
        _ => false,
    }
}

pub fn format_name(id: u32) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClipboardFormatNameW(id, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

unsafe fn read_global(h: HANDLE) -> Option<Vec<u8>> {
    if h.is_null() {
        return None;
    }
    let size = GlobalSize(h);
    if size == 0 {
        return None;
    }
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, size).to_vec();
    GlobalUnlock(h);
    Some(v)
}

unsafe fn read_emf(h: HANDLE) -> Option<Vec<u8>> {
    let n = GetEnhMetaFileBits(h, 0, null_mut());
    if n == 0 {
        return None;
    }
    let mut v = vec![0u8; n as usize];
    if GetEnhMetaFileBits(h, n, v.as_mut_ptr()) == 0 {
        return None;
    }
    Some(v)
}

/// Exe name of the clipboard owner (or foreground window as fallback).
fn owner_name() -> String {
    unsafe {
        let mut hwnd = GetClipboardOwner();
        if hwnd.is_null() {
            hwnd = GetForegroundWindow();
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return String::new();
        }
        let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if ph.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(ph, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(ph);
        if ok == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or("").to_string()
    }
}

/// Empty the clipboard. Returns true on success.
pub fn clear(hwnd: HWND) -> bool {
    if !open(hwnd) {
        return false;
    }
    unsafe {
        EmptyClipboard();
        CloseClipboard();
    }
    true
}

/// Put a stored payload back on the clipboard. Returns true on success.
pub fn restore(hwnd: HWND, p: &Payload) -> bool {
    if !open(hwnd) {
        return false;
    }
    unsafe {
        EmptyClipboard();
        for f in p {
            let id = if f.name.is_empty() { f.id } else { RegisterClipboardFormatW(wide(&f.name).as_ptr()) };
            if id == 0 {
                continue;
            }
            if id == CF_ENHMETAFILE {
                let h = SetEnhMetaFileBits(f.data.len() as u32, f.data.as_ptr());
                if !h.is_null() && SetClipboardData(id, h).is_null() {
                    DeleteEnhMetaFile(h);
                }
                continue;
            }
            let h = GlobalAlloc(GMEM_MOVEABLE, f.data.len().max(1));
            if h.is_null() {
                continue;
            }
            let dst = GlobalLock(h) as *mut u8;
            if dst.is_null() {
                GlobalFree(h);
                continue;
            }
            std::ptr::copy_nonoverlapping(f.data.as_ptr(), dst, f.data.len());
            GlobalUnlock(h);
            if SetClipboardData(id, h).is_null() {
                GlobalFree(h);
            }
        }
        CloseClipboard();
    }
    true
}
