use crate::clip::*;
use crate::util::{human_size, w};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::*;

#[derive(PartialEq, Clone, Copy)]
pub enum Kind {
    Text,
    Rich,
    Image,
    Files,
    Other,
}

/// Scaled copy of a clipboard image, ready for drawing.
pub struct Thumb {
    pub hbmp: HBITMAP,
    pub w: i32,
    pub h: i32,
}

impl Drop for Thumb {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.hbmp) };
    }
}

/// Everything the overlay needs to draw an item. Built once per item.
pub struct Preview {
    pub kind: Kind,
    /// Header line, e.g. "Image · 1920 × 1080".
    pub info: String,
    /// Body text (Text/Rich/Other) or parent folder (Files).
    pub text: Vec<u16>,
    pub thumb: Option<Thumb>,
    /// (file name, size or "folder")
    pub files: Vec<(Vec<u16>, Vec<u16>)>,
    /// One-line summary for the history list.
    pub line: Vec<u16>,
}

const MAX_TEXT: usize = 4000;
const MAX_FILES: usize = 500;

pub fn build(p: &Payload, thumb_max_w: i32) -> Preview {
    let get = |id: u32| p.iter().find(|f| f.id == id);
    let named = |n: &str| p.iter().any(|f| f.name == n);
    let mut pv = Preview { kind: Kind::Other, info: String::new(), text: Vec::new(), thumb: None, files: Vec::new(), line: Vec::new() };

    if let Some(f) = get(CF_HDROP) {
        files(&mut pv, &f.data);
    } else if let Some(f) = get(CF_UNICODETEXT) {
        let text: Vec<u16> = f.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        let rtf = named("Rich Text Format");
        let html = named("HTML Format");
        pv.kind = if rtf || html { Kind::Rich } else { Kind::Text };
        let label = match (rtf, html) {
            (true, true) => "Rich text (RTF, HTML)",
            (true, false) => "Rich text (RTF)",
            (false, true) => "Rich text (HTML)",
            _ => "Text",
        };
        let lines = text.iter().filter(|&&c| c == '\n' as u16).count() + 1;
        pv.info = format!("{} · {} chars · {} line{}", label, text.len(), lines, if lines == 1 { "" } else { "s" });
        let s = String::from_utf16_lossy(&text[..text.len().min(MAX_TEXT)]);
        let first = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        pv.line = w(&first.chars().take(120).map(|c| if c == '\t' { ' ' } else { c }).collect::<String>());
        pv.text = text[..text.len().min(MAX_TEXT)].to_vec();
    } else if let Some(f) = get(CF_DIBV5).or_else(|| get(CF_DIB)) {
        pv.kind = Kind::Image;
        let d = &f.data;
        if d.len() >= 16 {
            let iw = i32::from_le_bytes([d[4], d[5], d[6], d[7]]);
            let ih = i32::from_le_bytes([d[8], d[9], d[10], d[11]]).abs();
            let bpp = u16::from_le_bytes([d[14], d[15]]);
            pv.info = format!("Image · {} × {} · {}-bit", iw, ih, bpp);
            pv.line = w(&format!("Image {} × {}", iw, ih));
        }
        pv.thumb = make_thumb(d, thumb_max_w);
    } else {
        let total: u64 = p.iter().map(|f| f.data.len() as u64).sum();
        pv.info = format!("Other · {} formats · {}", p.len(), human_size(total));
        let mut s = String::new();
        for f in p {
            s += &format!("{}  —  {}\n", fmt_label(f), human_size(f.data.len() as u64));
        }
        pv.text = w(&s);
        pv.line = w(&p.first().map(fmt_label).unwrap_or_default());
    }
    pv
}

fn fmt_label(f: &Fmt) -> String {
    if !f.name.is_empty() {
        return f.name.clone();
    }
    let n = match f.id {
        1 => "Text", 4 => "SYLK", 5 => "DIF", 6 => "TIFF", 7 => "OEM text", 8 => "DIB", 10 => "Pen data",
        11 => "RIFF", 12 => "Wave", 13 => "Unicode text", 14 => "Enhanced metafile", 15 => "Files",
        16 => "Locale", 17 => "DIB v5",
        _ => return format!("Format {}", f.id),
    };
    n.to_string()
}

/// File list from a DROPFILES block.
pub fn parse_hdrop(d: &[u8]) -> Vec<String> {
    if d.len() < 20 {
        return Vec::new();
    }
    let off = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
    let wide = u32::from_le_bytes([d[16], d[17], d[18], d[19]]) != 0;
    if off >= d.len() {
        return Vec::new();
    }
    let body = &d[off..];
    if wide {
        let u: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        u.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    } else {
        body.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
    }
}

fn files(pv: &mut Preview, d: &[u8]) {
    pv.kind = Kind::Files;
    let paths = parse_hdrop(d);
    let (mut nfiles, mut ndirs, mut total) = (0, 0, 0u64);
    for p in paths.iter().take(MAX_FILES) {
        let name = Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
        let size = match std::fs::metadata(p) {
            Ok(m) if m.is_dir() => {
                ndirs += 1;
                "folder".to_string()
            }
            Ok(m) => {
                nfiles += 1;
                total += m.len();
                human_size(m.len())
            }
            Err(_) => "missing".to_string(),
        };
        pv.files.push((w(&name), w(&size)));
    }
    let mut parts = Vec::new();
    if nfiles > 0 {
        parts.push(format!("{} file{}", nfiles, if nfiles == 1 { "" } else { "s" }));
    }
    if ndirs > 0 {
        parts.push(format!("{} folder{}", ndirs, if ndirs == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        parts.push(format!("{} items", paths.len()));
    }
    pv.info = format!("Files · {} · {}", parts.join(", "), human_size(total));
    if let Some(parent) = paths.first().and_then(|p| Path::new(p).parent()) {
        pv.text = w(&parent.to_string_lossy());
    }
    if let Some((first, _)) = pv.files.first() {
        let name = String::from_utf16_lossy(first);
        pv.line = w(&if paths.len() > 1 { format!("{} (+{} more)", name, paths.len() - 1) } else { name });
    }
}

/// Scale a packed DIB down to at most `max_w` wide (and 2×max_w tall).
/// Only uncompressed DIBs whose pixel data fits inside `d` are accepted.
fn make_thumb(d: &[u8], max_w: i32) -> Option<Thumb> {
    if d.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let hs = u32_at(0) as usize;
    let iw = u32_at(4) as i32;
    let ih = (u32_at(8) as i32).unsigned_abs() as usize;
    let bpp = u16::from_le_bytes([d[14], d[15]]) as usize;
    let comp = u32_at(16);
    let clr_used = u32_at(32) as usize;
    if iw <= 0 || ih == 0 || hs < 40 || hs > d.len() || (comp != 0 && comp != 3) || ![1, 4, 8, 16, 24, 32].contains(&bpp) {
        return None;
    }
    let mut off = hs;
    if hs == 40 && comp == 3 {
        off += 12;
    }
    off += 4 * if clr_used > 0 { clr_used.min(256) } else if bpp <= 8 { 1 << bpp } else { 0 };
    let stride = (iw as usize * bpp).div_ceil(32) * 4;
    if off.checked_add(stride.checked_mul(ih)?)? > d.len() {
        return None;
    }
    let max_w = max_w.max(64) as f64;
    let scale = (max_w / iw as f64).min(max_w * 2.0 / ih as f64).min(1.0);
    let tw = ((iw as f64 * scale).round() as i32).max(1);
    let th = ((ih as f64 * scale).round() as i32).max(1);
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = tw;
        bmi.bmiHeader.biHeight = -th;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let dc = CreateCompatibleDC(null_mut());
        let hbmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if hbmp.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, hbmp);
        SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, null_mut());
        StretchDIBits(dc, 0, 0, tw, th, 0, 0, iw, ih as i32, d.as_ptr().add(off) as *const _, d.as_ptr() as *const BITMAPINFO, DIB_RGB_COLORS, SRCCOPY);
        SelectObject(dc, old);
        DeleteDC(dc);
        Some(Thumb { hbmp, w: tw, h: th })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16z(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
    }

    fn hdrop(paths: &[&str]) -> Vec<u8> {
        let mut d = vec![0u8; 20];
        d[0] = 20; // pFiles
        d[16] = 1; // fWide
        for p in paths {
            d.extend(utf16z(p));
        }
        d.extend([0, 0]);
        d
    }

    /// Packed 24-bit bottom-up DIB, w×h.
    fn dib(w: i32, h: i32) -> Vec<u8> {
        let stride = ((w * 24 + 31) / 32 * 4) as usize;
        let mut d = vec![0u8; 40];
        d[0..4].copy_from_slice(&40u32.to_le_bytes());
        d[4..8].copy_from_slice(&w.to_le_bytes());
        d[8..12].copy_from_slice(&h.to_le_bytes());
        d[12..14].copy_from_slice(&1u16.to_le_bytes());
        d[14..16].copy_from_slice(&24u16.to_le_bytes());
        d.extend(vec![0x80u8; stride * h as usize]);
        d
    }

    #[test]
    fn hdrop_parsing() {
        assert_eq!(parse_hdrop(&hdrop(&[r"C:\a.txt", r"C:\b c.bin"])), vec![r"C:\a.txt", r"C:\b c.bin"]);
        assert!(parse_hdrop(&[1, 2, 3]).is_empty());
        let mut bad = hdrop(&[r"C:\a"]);
        bad[0] = 200; // offset past end
        assert!(parse_hdrop(&bad).is_empty());
    }

    #[test]
    fn text_preview() {
        let p = vec![Fmt { id: CF_UNICODETEXT, name: String::new(), data: utf16z("\n  first line\nsecond") }];
        let pv = build(&p, 300);
        assert!(pv.kind == Kind::Text);
        assert_eq!(String::from_utf16_lossy(&pv.line), "first line");
        assert!(pv.info.contains("3 lines"));
    }

    #[test]
    fn rich_text_preview() {
        let p = vec![
            Fmt { id: CF_UNICODETEXT, name: String::new(), data: utf16z("hi") },
            Fmt { id: 0xC100, name: "Rich Text Format".into(), data: br"{\rtf1 hi}".to_vec() },
        ];
        let pv = build(&p, 300);
        assert!(pv.kind == Kind::Rich);
        assert!(pv.info.starts_with("Rich text (RTF)"));
    }

    #[test]
    fn files_preview_with_real_file() {
        let path = std::env::temp_dir().join(format!("unshufflz-pv-{}.txt", std::process::id()));
        std::fs::write(&path, vec![0u8; 2048]).unwrap();
        let ps = path.to_string_lossy().into_owned();
        let p = vec![Fmt { id: CF_HDROP, name: String::new(), data: hdrop(&[&ps, r"Z:\missing\x.bin"]) }];
        let pv = build(&p, 300);
        let _ = std::fs::remove_file(&path);
        assert!(pv.kind == Kind::Files);
        assert_eq!(pv.files.len(), 2);
        assert_eq!(String::from_utf16_lossy(&pv.files[0].1), "2.0 KB");
        assert_eq!(String::from_utf16_lossy(&pv.files[1].1), "missing");
        assert!(String::from_utf16_lossy(&pv.line).ends_with("(+1 more)"));
    }

    #[test]
    fn image_thumb_scaled() {
        let p = vec![Fmt { id: CF_DIB, name: String::new(), data: dib(1000, 500) }];
        let pv = build(&p, 200);
        assert!(pv.kind == Kind::Image);
        assert!(pv.info.contains("1000 × 500"));
        let t = pv.thumb.expect("thumb");
        assert_eq!((t.w, t.h), (200, 100));
    }

    #[test]
    fn small_image_not_upscaled() {
        let t = make_thumb(&dib(10, 5), 200).unwrap();
        assert_eq!((t.w, t.h), (10, 5));
    }

    #[test]
    fn truncated_or_bad_dib_rejected() {
        let d = dib(100, 100);
        assert!(make_thumb(&d[..d.len() - 1], 200).is_none());
        assert!(make_thumb(&d[..30], 200).is_none());
        let mut c = d.clone();
        c[16] = 1; // BI_RLE8
        assert!(make_thumb(&c, 200).is_none());
        let mut n = d;
        n[4..8].copy_from_slice(&(-5i32).to_le_bytes());
        assert!(make_thumb(&n, 200).is_none());
    }
}
