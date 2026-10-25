//! Win+S: save the clipboard image as PNG and build a "copied file" clipboard payload.

use crate::clip::{Fmt, Payload, CF_DIB, CF_DIBV5, CF_HDROP};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath};

/// Pixel layout of a packed DIB (CF_DIB / CF_DIBV5).
#[derive(Debug, PartialEq)]
pub struct DibInfo {
    pub width: i32,
    /// Negative = top-down rows.
    pub height: i32,
    pub bpp: usize,
    pub compression: u32,
    /// Offset of the pixel data in the buffer.
    pub bits_offset: usize,
    /// Pixel data is fully inside the buffer (only checked for BI_RGB / BI_BITFIELDS).
    pub complete: bool,
}

/// Parse a packed DIB header. None if the header itself is broken.
pub fn dib_info(d: &[u8]) -> Option<DibInfo> {
    if d.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let hs = u32_at(0) as usize;
    let width = u32_at(4) as i32;
    let height = u32_at(8) as i32;
    let bpp = u16::from_le_bytes([d[14], d[15]]) as usize;
    let compression = u32_at(16);
    let clr_used = u32_at(32) as usize;
    if width <= 0 || height == 0 || height == i32::MIN || hs < 40 || hs > d.len() {
        return None;
    }
    let mut off = hs;
    if hs == 40 && compression == 3 {
        off += 12; // BI_BITFIELDS masks after a BITMAPINFOHEADER
    }
    off += 4 * if clr_used > 0 { clr_used.min(256) } else if bpp <= 8 { 1 << bpp } else { 0 };
    let uncompressed = (compression == 0 || compression == 3) && [1, 4, 8, 16, 24, 32].contains(&bpp);
    let stride = (width as usize * bpp).div_ceil(32) * 4;
    let complete = uncompressed
        && stride.checked_mul(height.unsigned_abs() as usize).and_then(|n| n.checked_add(off)).is_some_and(|end| end <= d.len());
    Some(DibInfo { width, height, bpp, compression, bits_offset: off, complete })
}

/// Convert a packed DIB to top-down RGBA. 24/32-bit images are read directly
/// (32-bit keeps its alpha unless all alpha bytes are 0); others are converted by GDI.
pub fn dib_to_rgba(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let info = dib_info(d)?;
    if !info.complete {
        return None;
    }
    let (w, h) = (info.width as usize, info.height.unsigned_abs() as usize);
    let bottom_up = info.height > 0;
    let stride = (w * info.bpp).div_ceil(32) * 4;
    let bits = &d[info.bits_offset..];
    let standard_masks = info.compression == 0
        || (d.len() >= 52 && d[40..52] == [0, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0]);
    let mut out = vec![0u8; w * h * 4];
    match info.bpp {
        32 if standard_masks => {
            for y in 0..h {
                let src = &bits[(if bottom_up { h - 1 - y } else { y }) * stride..];
                for x in 0..w {
                    let p = &src[x * 4..x * 4 + 4];
                    out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            if out.chunks_exact(4).all(|p| p[3] == 0) {
                out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            }
        }
        24 if info.compression == 0 => {
            for y in 0..h {
                let src = &bits[(if bottom_up { h - 1 - y } else { y }) * stride..];
                for x in 0..w {
                    let p = &src[x * 3..x * 3 + 3];
                    out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
        }
        _ => unsafe {
            // Let GDI convert palettes, 16-bit and bitfields to 32-bit BGRX.
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let mut px = null_mut();
            let dc = CreateCompatibleDC(null_mut());
            let hbmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut px, null_mut(), 0);
            if hbmp.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, hbmp);
            SetStretchBltMode(dc, COLORONCOLOR);
            StretchDIBits(dc, 0, 0, w as i32, h as i32, 0, 0, w as i32, h as i32, bits.as_ptr() as *const _, d.as_ptr() as *const BITMAPINFO, DIB_RGB_COLORS, SRCCOPY);
            GdiFlush();
            let src = std::slice::from_raw_parts(px as *const u8, w * h * 4);
            for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                o.copy_from_slice(&[p[2], p[1], p[0], 255]);
            }
            SelectObject(dc, old);
            DeleteObject(hbmp);
            DeleteDC(dc);
        },
    }
    Some((w as u32, h as u32, out))
}

/// Encode RGBA as PNG (RGB if fully opaque, which makes smaller files).
pub fn encode_png(w: u32, h: u32, rgba: &[u8], out: impl Write) -> io::Result<()> {
    let opaque = rgba.chunks_exact(4).all(|p| p[3] == 255);
    let mut enc = png::Encoder::new(out, w, h);
    enc.set_depth(png::BitDepth::Eight);
    let err = |e: png::EncodingError| io::Error::new(io::ErrorKind::Other, e.to_string());
    if opaque {
        enc.set_color(png::ColorType::Rgb);
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        enc.write_header().map_err(err)?.write_image_data(&rgb).map_err(err)
    } else {
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().map_err(err)?.write_image_data(rgba).map_err(err)
    }
}

/// "Unshufflz_yyyyMMdd_HHmmss.png", with "_2", "_3"… for repeats in the same second.
pub fn file_name(t: &SYSTEMTIME, n: u32) -> String {
    let base = format!("Unshufflz_{:04}{:02}{:02}_{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    if n <= 1 { format!("{}.png", base) } else { format!("{}_{}.png", base, n) }
}

/// The user's Pictures folder (follows OneDrive / folder redirection).
pub fn pictures_dir() -> PathBuf {
    unsafe {
        let mut p = null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Pictures, 0, null_mut(), &mut p) >= 0 && !p.is_null() {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            CoTaskMemFree(p as *const _);
            return PathBuf::from(s);
        }
        if !p.is_null() {
            CoTaskMemFree(p as *const _);
        }
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Pictures")
}

/// Default folder for Win+S pictures: Pictures\Clipboard.
pub fn default_dir() -> PathBuf {
    pictures_dir().join("Clipboard")
}

/// Folder for Win+S pictures from the `PicturesDir` setting (empty = default).
pub fn target_dir(setting: &str) -> PathBuf {
    if setting.trim().is_empty() { default_dir() } else { PathBuf::from(setting.trim()) }
}

/// Save the image in `p` into `dir` as a new PNG file and return its path.
/// Uses the "PNG" clipboard format as-is when present, else converts the DIB.
pub fn save_png(p: &Payload, dir: &Path) -> io::Result<PathBuf> {
    let png_bytes = p.iter().find(|f| f.name.eq_ignore_ascii_case("PNG")).map(|f| &f.data);
    let rgba = if png_bytes.is_none() {
        let dib = p.iter().find(|f| f.id == CF_DIBV5).or_else(|| p.iter().find(|f| f.id == CF_DIB));
        let dib = dib.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no image"))?;
        Some(dib_to_rgba(&dib.data).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unsupported bitmap"))?)
    } else {
        None
    };
    fs::create_dir_all(dir)?;
    let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    for n in 1..1000 {
        let path = dir.join(file_name(&now, n));
        // create_new: never overwrite an existing picture
        let file = match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let r = (|| {
            let mut w = BufWriter::new(&file);
            match (&png_bytes, &rgba) {
                (Some(b), _) => w.write_all(b)?,
                (None, Some((iw, ih, px))) => encode_png(*iw, *ih, px, &mut w)?,
                _ => unreachable!(),
            }
            w.flush()?;
            file.sync_all()
        })();
        if let Err(e) = r {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        return Ok(path);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files this second"))
}

/// Clipboard payload for "copied file" (as Explorer's Copy): CF_HDROP + Preferred DropEffect = copy.
pub fn file_payload(path: &Path) -> Payload {
    let mut d = vec![0u8; 20];
    d[0] = 20; // pFiles: list starts after the 20-byte DROPFILES header
    d[16] = 1; // fWide: UTF-16 names
    for c in path.to_string_lossy().encode_utf16().chain([0, 0]) {
        d.extend_from_slice(&c.to_le_bytes());
    }
    vec![
        Fmt { id: CF_HDROP, name: String::new(), data: d },
        Fmt { id: 0, name: "Preferred DropEffect".into(), data: 1u32.to_le_bytes().to_vec() },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: i32, h: i32, bpp: u16, comp: u32) -> Vec<u8> {
        let mut d = vec![0u8; 40];
        d[0..4].copy_from_slice(&40u32.to_le_bytes());
        d[4..8].copy_from_slice(&w.to_le_bytes());
        d[8..12].copy_from_slice(&h.to_le_bytes());
        d[12..14].copy_from_slice(&1u16.to_le_bytes());
        d[14..16].copy_from_slice(&bpp.to_le_bytes());
        d[16..20].copy_from_slice(&comp.to_le_bytes());
        d
    }

    #[test]
    fn rgb24_bottom_up() {
        // 2×2, bottom-up: first row in memory is the bottom row. Stride 8 (6 + 2 padding).
        let mut d = header(2, 2, 24, 0);
        d.extend([0, 0, 255, 0, 255, 0, 0, 0]); // bottom: red, green (BGR)
        d.extend([255, 0, 0, 255, 255, 255, 0, 0]); // top: blue, white
        let (w, h, px) = dib_to_rgba(&d).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(px, vec![0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn rgb32_alpha_rules() {
        let mut d = header(1, -2, 32, 0); // top-down
        d.extend([10, 20, 30, 0, 40, 50, 60, 0]); // alpha all zero -> opaque
        let (_, _, px) = dib_to_rgba(&d).unwrap();
        assert_eq!(px, vec![30, 20, 10, 255, 60, 50, 40, 255]);
        let mut d = header(1, -2, 32, 0);
        d.extend([10, 20, 30, 128, 40, 50, 60, 255]); // real alpha kept
        let (_, _, px) = dib_to_rgba(&d).unwrap();
        assert_eq!(px, vec![30, 20, 10, 128, 60, 50, 40, 255]);
    }

    #[test]
    fn palette_image_via_gdi() {
        // 8-bit, 2-color palette, 1×1 pixel using color 1 (green).
        let mut d = header(1, 1, 8, 0);
        d[32..36].copy_from_slice(&2u32.to_le_bytes());
        d.extend([0, 0, 0, 0, 0, 255, 0, 0]); // palette: black, green (BGRX)
        d.extend([1, 0, 0, 0]); // pixel + padding
        let (_, _, px) = dib_to_rgba(&d).unwrap();
        assert_eq!(px, vec![0, 255, 0, 255]);
    }

    #[test]
    fn broken_dibs_rejected() {
        let mut d = header(4, 4, 24, 0);
        d.extend([0u8; 10]); // too little pixel data
        assert!(dib_to_rgba(&d).is_none());
        assert!(dib_to_rgba(&header(4, 4, 24, 0)[..20]).is_none());
        let mut c = header(1, 1, 24, 1); // RLE8
        c.extend([0u8; 4]);
        assert!(dib_to_rgba(&c).is_none());
    }

    #[test]
    fn png_roundtrip() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 128];
        let mut out = Vec::new();
        encode_png(2, 1, &rgba, &mut out).unwrap();
        let mut r = png::Decoder::new(&out[..]).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgba));
        assert_eq!(&buf[..8], &rgba[..]);
        // opaque image is stored as RGB
        let mut out = Vec::new();
        encode_png(1, 1, &[1, 2, 3, 255], &mut out).unwrap();
        assert_eq!(png::Decoder::new(&out[..]).read_info().unwrap().info().color_type, png::ColorType::Rgb);
    }

    #[test]
    fn target_folder_setting() {
        assert!(target_dir("").ends_with("Clipboard"));
        assert!(target_dir("  ").ends_with("Clipboard"));
        assert_eq!(target_dir(r"D:\Shots"), PathBuf::from(r"D:\Shots"));
    }

    #[test]
    fn names() {
        let t = SYSTEMTIME { wYear: 2026, wMonth: 3, wDayOfWeek: 0, wDay: 7, wHour: 9, wMinute: 5, wSecond: 1, wMilliseconds: 0 };
        assert_eq!(file_name(&t, 1), "Unshufflz_20260307_090501.png");
        assert_eq!(file_name(&t, 2), "Unshufflz_20260307_090501_2.png");
    }

    #[test]
    fn save_and_file_payload() {
        let dir = std::env::temp_dir().join(format!("unshufflz-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut dib = header(1, 1, 24, 0);
        dib.extend([0, 0, 255, 0]);
        let p = vec![Fmt { id: CF_DIB, name: String::new(), data: dib }];
        let a = save_png(&p, &dir).unwrap();
        let b = save_png(&p, &dir).unwrap(); // same second -> new name, no overwrite
        assert_ne!(a, b);
        assert!(png::Decoder::new(fs::File::open(&a).unwrap()).read_info().is_ok());
        // a PNG format on the clipboard is written as-is
        let p2 = vec![Fmt { id: 0xC123, name: "PNG".into(), data: fs::read(&a).unwrap() }];
        let c = save_png(&p2, &dir).unwrap();
        assert_eq!(fs::read(&c).unwrap(), fs::read(&a).unwrap());
        let fp = file_payload(&a);
        assert_eq!(crate::preview::parse_hdrop(&fp[0].data), vec![a.to_string_lossy().into_owned()]);
        assert_eq!(fp[1].data, vec![1, 0, 0, 0]);
        let _ = fs::remove_dir_all(&dir);
    }
}
