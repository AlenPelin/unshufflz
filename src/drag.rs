//! Drag files out of the overlay (file panel) into Explorer, a browser, etc.

use windows::core::HSTRING;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{IDropSource, OleInitialize, DROPEFFECT_COPY, DROPEFFECT_LINK};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject, SHDoDragDrop};

/// OLE drag and drop needs OleInitialize on the thread that starts the drag (main thread).
pub fn init() {
    unsafe {
        let _ = OleInitialize(None);
    }
}

/// Shell data object for `pidls` (absolute PIDLs): CF_HDROP, Shell IDList Array, file contents, …
/// The parent must be the desktop (an empty ID list), not NULL: with NULL the object is empty.
fn data_object(pidls: &[*const ITEMIDLIST]) -> Option<IDataObject> {
    let desktop = [0u16; 1]; // empty ITEMIDLIST (cb = 0)
    unsafe { SHCreateDataObject(Some(desktop.as_ptr() as *const ITEMIDLIST), Some(pidls), None::<&IDataObject>).ok() }
}

/// Run a shell drag of `paths` (blocks until the drop). Missing files are skipped.
/// Only copy and link are offered, so a drop never moves the original file.
pub fn drag_files(hwnd: *mut core::ffi::c_void, paths: &[String]) {
    unsafe {
        // Absolute PIDLs under the desktop: works for files from different folders.
        let pidls: Vec<*const ITEMIDLIST> =
            paths.iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.as_str())) as *const ITEMIDLIST).filter(|p| !p.is_null()).collect();
        if !pidls.is_empty() {
            if let Some(obj) = data_object(&pidls) {
                // No drop source given: the shell uses its own (with drag images).
                let r = SHDoDragDrop(HWND(hwnd), &obj, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_LINK);
                crate::util::debug(|| format!("main drag {} files: {:?}", pidls.len(), r));
            }
        }
        for p in pidls {
            ILFree(Some(p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::FORMATETC;

    #[test]
    fn data_object_offers_files_from_two_folders() {
        init();
        let a = std::env::temp_dir().join(format!("unshufflz-drag-{}.txt", std::process::id()));
        std::fs::write(&a, b"x").unwrap();
        let b = std::env::current_exe().unwrap(); // another folder
        unsafe {
            let pidls: Vec<*const ITEMIDLIST> =
                [&a, &b].iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.to_string_lossy().as_ref())) as *const ITEMIDLIST).collect();
            let obj = data_object(&pidls).expect("data object");
            let q = FORMATETC { cfFormat: 15, ptd: std::ptr::null_mut(), dwAspect: 1, lindex: -1, tymed: 1 }; // CF_HDROP, HGLOBAL
            let hdrop = obj.QueryGetData(&q);
            for p in pidls {
                ILFree(Some(p));
            }
            let _ = std::fs::remove_file(&a);
            assert_eq!(hdrop.0, 0, "CF_HDROP offered");
        }
    }
}
