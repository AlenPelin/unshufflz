//! Unshufflz setup: per-user install / uninstall (no admin rights needed).
//!
//!   unshufflz-setup.exe                 install (asks questions)
//!   unshufflz-setup.exe /S              silent install, autostart on
//!   unshufflz-setup.exe /S /noautostart silent install, autostart off
//!   uninstall.exe /uninstall              uninstall (asks questions)
//!   uninstall.exe /uninstall /S [/purge]  silent uninstall; /purge also deletes history and settings

#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::{OpenProcess, Sleep, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static APP_EXE: &[u8] = include_bytes!(env!("APP_EXE"));
const VERSION: &str = env!("APP_VERSION");
const TITLE: &str = "Unshufflz Setup";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Unshufflz";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_WINDOW_CLASS: &str = "UnshufflzOverlay";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Default, PartialEq)]
struct Args {
    uninstall: bool,
    silent: bool,
    purge: bool,
    no_autostart: bool,
    /// Internal: this is the temp copy started by uninstall.exe.
    temp_copy: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Args {
    let mut a = Args::default();
    for s in args {
        match s.trim_start_matches(['/', '-']).to_ascii_lowercase().as_str() {
            "uninstall" => a.uninstall = true,
            "s" | "silent" => a.silent = true,
            "purge" => a.purge = true,
            "noautostart" => a.no_autostart = true,
            "tempcopy" => a.temp_copy = true,
            _ => {}
        }
    }
    a
}

struct Paths {
    dir: PathBuf,
    exe: PathBuf,
    uninstaller: PathBuf,
    shortcut: PathBuf,
    data: PathBuf,
}

fn paths_from(local: &Path, roaming: &Path) -> Paths {
    let dir = local.join("Programs").join("Unshufflz");
    Paths {
        exe: dir.join("unshufflz.exe"),
        uninstaller: dir.join("uninstall.exe"),
        shortcut: roaming.join(r"Microsoft\Windows\Start Menu\Programs\Unshufflz.lnk"),
        data: local.join("Unshufflz"),
        dir,
    }
}

fn paths() -> Paths {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from).unwrap_or_default();
    paths_from(&env("LOCALAPPDATA"), &env("APPDATA"))
}

fn main() {
    let args = parse_args(std::env::args().skip(1));
    let r = if args.uninstall { uninstall(&args) } else { install(&args) };
    if args.temp_copy {
        if let Ok(me) = std::env::current_exe() {
            delete_later(&me);
        }
    }
    if let Err(e) = r {
        if !args.silent {
            message(&format!("Setup failed:\n\n{}", e), MB_ICONERROR);
        }
        std::process::exit(1);
    }
}

// ---------- install ----------

fn install(a: &Args) -> io::Result<()> {
    let p = paths();
    let mut autostart = !a.no_autostart;
    if !a.silent {
        let q = if p.exe.exists() {
            format!("Update Unshufflz to version {}?\n\nFolder: {}", VERSION, p.dir.display())
        } else {
            format!("Install Unshufflz {} for the current user?\n\nFolder: {}", VERSION, p.dir.display())
        };
        if !ask(&q, false) {
            return Ok(());
        }
        autostart = ask("Start Unshufflz automatically when you sign in to Windows?", false);
    }
    stop_app();
    fs::create_dir_all(&p.dir)?;
    replace_with_retry(&p.exe, APP_EXE)?;
    let me = std::env::current_exe()?;
    if !same_path(&me, &p.uninstaller) {
        replace_with_retry(&p.uninstaller, &fs::read(&me)?)?;
    }
    if let Err(e) = create_shortcut(&p.shortcut, &p.exe) {
        // Not fatal: the app works without the Start menu entry.
        if !a.silent {
            message(&format!("Could not create the Start menu shortcut:\n{}", e), MB_ICONWARNING);
        }
    }
    register_uninstall(&p, (APP_EXE.len() + me.metadata().map(|m| m.len() as usize).unwrap_or(0)) as u32 / 1024);
    if autostart {
        reg_set_str(RUN_KEY, "Unshufflz", &format!("\"{}\"", p.exe.display()));
    } else {
        reg_delete_value(RUN_KEY, "Unshufflz");
    }
    Command::new(&p.exe).spawn()?;
    if !a.silent {
        message(
            "Unshufflz is installed and running.\n\n\
             Hold the Win key to see your clipboard history.\n\
             Hold Win and press V to go back, C to go forward.\n\
             Settings are in the tray icon menu.",
            MB_ICONINFORMATION,
        );
    }
    Ok(())
}

/// Write `<path>.new`, then rename it over `path`, retrying while the old file is locked.
fn replace_with_retry(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("new");
    fs::write(&tmp, data)?;
    let mut last = None;
    for _ in 0..50 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        unsafe { Sleep(100) };
    }
    let _ = fs::remove_file(&tmp);
    Err(last.unwrap())
}

fn register_uninstall(p: &Paths, size_kb: u32) {
    let u = p.uninstaller.display();
    reg_set_str(UNINSTALL_KEY, "DisplayName", "Unshufflz");
    reg_set_str(UNINSTALL_KEY, "DisplayVersion", VERSION);
    reg_set_str(UNINSTALL_KEY, "Publisher", "Unshufflz");
    reg_set_str(UNINSTALL_KEY, "DisplayIcon", &p.exe.display().to_string());
    reg_set_str(UNINSTALL_KEY, "InstallLocation", &p.dir.display().to_string());
    reg_set_str(UNINSTALL_KEY, "UninstallString", &format!("\"{}\" /uninstall", u));
    reg_set_str(UNINSTALL_KEY, "QuietUninstallString", &format!("\"{}\" /uninstall /S", u));
    reg_set_dword(UNINSTALL_KEY, "NoModify", 1);
    reg_set_dword(UNINSTALL_KEY, "NoRepair", 1);
    reg_set_dword(UNINSTALL_KEY, "EstimatedSize", size_kb);
}

fn create_shortcut(lnk: &Path, target: &Path) -> windows::core::Result<()> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    if let Some(dir) = lnk.parent() {
        let _ = fs::create_dir_all(dir);
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(target.to_string_lossy().as_ref()))?;
        if let Some(dir) = target.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.to_string_lossy().as_ref()))?;
        }
        link.SetDescription(&HSTRING::from("Clipboard history overlay (hold the Win key)"))?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(lnk.to_string_lossy().as_ref()), true)?;
    }
    Ok(())
}

// ---------- uninstall ----------

fn uninstall(a: &Args) -> io::Result<()> {
    let p = paths();
    let me = std::env::current_exe()?;
    // A running exe can't be deleted. When started from the install folder,
    // continue from a temp copy (which deletes itself at the end).
    if !a.temp_copy && is_inside(&me, &p.dir) {
        let tmp = std::env::temp_dir().join(format!("unshufflz-uninstall-{}.exe", std::process::id()));
        fs::copy(&me, &tmp)?;
        let mut c = Command::new(&tmp);
        c.args(["/uninstall", "/tempcopy"]);
        if a.silent {
            c.arg("/S");
        }
        if a.purge {
            c.arg("/purge");
        }
        c.spawn()?;
        return Ok(());
    }
    let mut purge = a.purge;
    if !a.silent {
        if !ask("Remove Unshufflz from this computer?", false) {
            return Ok(());
        }
        purge = ask("Also delete your saved clipboard history and settings?", true);
    }
    stop_app();
    reg_delete_value(RUN_KEY, "Unshufflz");
    unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(UNINSTALL_KEY).as_ptr()) };
    let _ = fs::remove_file(&p.shortcut);
    remove_dir_with_retry(&p.dir)?;
    if purge {
        let _ = fs::remove_dir_all(&p.data);
    }
    if !a.silent {
        message("Unshufflz was removed.", MB_ICONINFORMATION);
    }
    Ok(())
}

/// The uninstall.exe that started us may still be exiting, so retry for up to 5 s.
fn remove_dir_with_retry(dir: &Path) -> io::Result<()> {
    let mut last = Ok(());
    for _ in 0..50 {
        if !dir.exists() {
            return Ok(());
        }
        last = fs::remove_dir_all(dir);
        if last.is_ok() {
            return Ok(());
        }
        unsafe { Sleep(100) };
    }
    last
}

/// Delete a file a moment after this process exits (used by the temp copy).
fn delete_later(path: &Path) {
    let _ = Command::new("cmd")
        .raw_arg(format!("/c ping 127.0.0.1 -n 3 > nul & del /f /q \"{}\"", path.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

// ---------- helpers ----------

/// Ask the running app to exit (WM_CLOSE → it saves pending history) and wait for it.
fn stop_app() {
    unsafe {
        let class = wide(APP_WINDOW_CLASS);
        for _ in 0..5 {
            let h = FindWindowW(class.as_ptr(), null());
            if h.is_null() {
                return;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            let ph = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            PostMessageW(h, WM_CLOSE, 0, 0);
            if ph.is_null() {
                Sleep(1000);
            } else {
                WaitForSingleObject(ph, 5000);
                CloseHandle(ph);
            }
        }
    }
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().to_lowercase().replace('/', "\\")
}

fn same_path(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

fn is_inside(file: &Path, dir: &Path) -> bool {
    norm(file).starts_with(&(norm(dir).trim_end_matches('\\').to_string() + "\\"))
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

fn message(text: &str, icon: u32) {
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(TITLE).as_ptr(), MB_OK | icon) };
}

fn ask(text: &str, default_no: bool) -> bool {
    let flags = MB_YESNO | MB_ICONQUESTION | if default_no { MB_DEFBUTTON2 } else { 0 };
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(TITLE).as_ptr(), flags) == IDYES }
}

fn reg_set_str(key: &str, name: &str, value: &str) {
    let v = wide(value);
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), REG_SZ, v.as_ptr() as *const _, (v.len() * 2) as u32);
    }
}

fn reg_set_dword(key: &str, name: &str, value: u32) {
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), REG_DWORD, &value as *const u32 as *const _, 4);
    }
}

fn reg_delete_value(key: &str, name: &str) {
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Args {
        parse_args(s.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        assert_eq!(args(&[]), Args::default());
        let a = args(&["/UNINSTALL", "-s", "/purge", "/tempcopy", "/whatever"]);
        assert!(a.uninstall && a.silent && a.purge && a.temp_copy && !a.no_autostart);
        assert!(args(&["/S", "/NoAutostart"]).no_autostart);
    }

    #[test]
    fn install_paths() {
        let p = paths_from(Path::new(r"C:\U\AppData\Local"), Path::new(r"C:\U\AppData\Roaming"));
        assert_eq!(p.exe, Path::new(r"C:\U\AppData\Local\Programs\Unshufflz\unshufflz.exe"));
        assert_eq!(p.uninstaller, Path::new(r"C:\U\AppData\Local\Programs\Unshufflz\uninstall.exe"));
        assert_eq!(p.data, Path::new(r"C:\U\AppData\Local\Unshufflz"));
        assert_eq!(p.shortcut, Path::new(r"C:\U\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Unshufflz.lnk"));
    }

    #[test]
    fn path_checks() {
        let dir = Path::new(r"C:\X\Programs\Unshufflz");
        assert!(is_inside(Path::new(r"c:\x\programs\unshufflz\uninstall.exe"), dir));
        assert!(!is_inside(Path::new(r"C:\X\Programs\Unshufflz2\uninstall.exe"), dir));
        assert!(same_path(Path::new("C:/A/b.exe"), Path::new(r"c:\a\B.EXE")));
    }

    #[test]
    fn embedded_app_is_an_exe() {
        assert!(APP_EXE.len() > 1000);
        assert_eq!(&APP_EXE[..2], b"MZ");
    }
}
