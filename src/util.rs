use std::io::{self, Write};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// Rename `from` over `to` with write-through, so the switch is on disk when this returns.
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    let f = wide(&from.to_string_lossy());
    let t = wide(&to.to_string_lossy());
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Write `<path>.tmp`, flush it to disk, then replace `path` with it.
pub fn write_file_safe(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let r = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        replace_file(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// UTF-16 string with trailing NUL, for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 string without NUL, for DrawTextW with explicit length.
pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn human_size(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{} B", n)
    } else if f < K * K {
        format!("{:.1} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / (K * K))
    } else {
        format!("{:.2} GB", f / (K * K * K))
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Local time: "HH:MM:SS" for today, "DD.MM.YYYY HH:MM" otherwise.
pub fn format_time(unix: u64) -> String {
    unsafe {
        let ft100 = (unix + 11_644_473_600) * 10_000_000;
        let ft = FILETIME { dwLowDateTime: ft100 as u32, dwHighDateTime: (ft100 >> 32) as u32 };
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut lt: SYSTEMTIME = std::mem::zeroed();
        let mut now: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut lt) == 0
        {
            return String::new();
        }
        GetLocalTime(&mut now);
        if lt.wYear == now.wYear && lt.wMonth == now.wMonth && lt.wDay == now.wDay {
            format!("{:02}:{:02}:{:02}", lt.wHour, lt.wMinute, lt.wSecond)
        } else {
            format!("{:02}.{:02}.{} {:02}:{:02}", lt.wDay, lt.wMonth, lt.wYear, lt.wHour, lt.wMinute)
        }
    }
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

/// %LOCALAPPDATA%\Unshufflz
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Unshufflz")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 << 20), "5.0 MB");
        assert_eq!(human_size(3 << 30), "3.00 GB");
    }

    #[test]
    fn wide_has_nul() {
        assert_eq!(wide("ab"), vec![97, 98, 0]);
        assert_eq!(w("ab"), vec![97, 98]);
    }

    #[test]
    fn time_today_is_short() {
        assert_eq!(format_time(now_unix()).len(), 8);
        assert!(format_time(0).contains("1970"));
    }
}
