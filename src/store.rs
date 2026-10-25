//! Disk store under %LOCALAPPDATA%\Unshufflz\store
//!
//! * `<id:016x>.item` – one immutable file per item (all formats), named by content hash.
//! * `index.dat`      – order of items plus time and source app.
//!
//! Every file ends with an xxh3 checksum of its body. Files are written to `*.tmp`,
//! flushed to disk, then renamed over the target with MOVEFILE_WRITE_THROUGH,
//! so a crash leaves either the old or the new file, never a broken one.
//! Item files are written before the index that references them.

use crate::clip::{Fmt, Payload};
use crate::util::wide;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
use xxhash_rust::xxh3::{xxh3_64, Xxh3};

const ITEM_MAGIC: &[u8; 4] = b"CLPI";
const INDEX_MAGIC: &[u8; 4] = b"CLPX";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub time: u64,
    pub source: String,
}

enum Job {
    Item(u64, Arc<Payload>),
    Index(Vec<Entry>),
    Quit,
}

/// Background writer. Jobs run in order; only the newest queued index is written.
pub struct Store {
    tx: Option<Sender<Job>>,
    th: Option<JoinHandle<()>>,
}

impl Store {
    /// After an item file is on disk, `saved_msg` (lParam = id) is posted to `hwnd`.
    pub fn start(dir: PathBuf, hwnd: isize, saved_msg: u32) -> Store {
        let _ = fs::create_dir_all(&dir);
        let (tx, rx) = channel::<Job>();
        let th = std::thread::spawn(move || {
            let mut quit = false;
            while !quit {
                let Ok(first) = rx.recv() else { break };
                let mut batch = vec![first];
                while let Ok(j) = rx.try_recv() {
                    batch.push(j);
                }
                let last_index = batch.iter().rposition(|j| matches!(j, Job::Index(_)));
                for (i, job) in batch.into_iter().enumerate() {
                    match job {
                        Job::Item(id, p) => {
                            if write_item(&dir, id, &p).is_ok() {
                                unsafe { PostMessageW(hwnd as _, saved_msg, 0, id as isize) };
                            }
                        }
                        Job::Index(e) if Some(i) == last_index => {
                            if write_index(&dir, &e).is_ok() {
                                gc(&dir, &e);
                            }
                        }
                        Job::Index(_) => {}
                        Job::Quit => quit = true,
                    }
                }
            }
        });
        Store { tx: Some(tx), th: Some(th) }
    }

    pub fn save_item(&self, id: u64, p: Arc<Payload>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job::Item(id, p));
        }
    }

    pub fn save_index(&self, e: Vec<Entry>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job::Index(e));
        }
    }

    /// Finish pending writes and stop the thread. Safe to call twice.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Job::Quit);
        }
        if let Some(th) = self.th.take() {
            let _ = th.join();
        }
    }
}

pub fn item_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{:016x}.item", id))
}

/// Writes body + checksum and feeds the checksum hasher at the same time.
struct HashWriter<W: Write> {
    w: W,
    h: Xxh3,
}

impl<W: Write> HashWriter<W> {
    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        self.h.update(b);
        self.w.write_all(b)
    }
}

fn write_atomic(path: &Path, body: impl FnOnce(&mut HashWriter<BufWriter<&File>>) -> io::Result<()>) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let r = (|| {
        let file = File::create(&tmp)?;
        {
            let mut hw = HashWriter { w: BufWriter::with_capacity(64 * 1024, &file), h: Xxh3::new() };
            body(&mut hw)?;
            let sum = hw.h.digest();
            hw.w.write_all(&sum.to_le_bytes())?;
            hw.w.flush()?;
        }
        file.sync_all()?;
        drop(file);
        replace(&tmp, path)
    })();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}

fn replace(from: &Path, to: &Path) -> io::Result<()> {
    let f = wide(&from.to_string_lossy());
    let t = wide(&to.to_string_lossy());
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn write_item(dir: &Path, id: u64, p: &Payload) -> io::Result<()> {
    let path = item_path(dir, id);
    if path.exists() {
        return Ok(()); // immutable, already complete
    }
    write_atomic(&path, |hw| {
        hw.put(ITEM_MAGIC)?;
        hw.put(&VERSION.to_le_bytes())?;
        hw.put(&(p.len() as u32).to_le_bytes())?;
        for f in p {
            hw.put(&f.id.to_le_bytes())?;
            hw.put(&(f.name.len() as u16).to_le_bytes())?;
            hw.put(f.name.as_bytes())?;
            hw.put(&(f.data.len() as u64).to_le_bytes())?;
            hw.put(&f.data)?;
        }
        Ok(())
    })
}

pub fn write_index(dir: &Path, e: &[Entry]) -> io::Result<()> {
    write_atomic(&dir.join("index.dat"), |hw| {
        hw.put(INDEX_MAGIC)?;
        hw.put(&VERSION.to_le_bytes())?;
        hw.put(&(e.len() as u32).to_le_bytes())?;
        for x in e {
            hw.put(&x.id.to_le_bytes())?;
            hw.put(&x.time.to_le_bytes())?;
            let s = x.source.as_bytes();
            let s = &s[..s.len().min(u16::MAX as usize)];
            hw.put(&(s.len() as u16).to_le_bytes())?;
            hw.put(s)?;
        }
        Ok(())
    })
}

/// Delete item files not in the index, and leftover temp files.
fn gc(dir: &Path, e: &[Entry]) {
    let keep: HashSet<u64> = e.iter().map(|x| x.id).collect();
    let Ok(rd) = fs::read_dir(dir) else { return };
    for f in rd.flatten() {
        let p = f.path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("item") => {
                let id = p.file_stem().and_then(|s| s.to_str()).and_then(|s| u64::from_str_radix(s, 16).ok());
                if id.is_none_or(|id| !keep.contains(&id)) {
                    let _ = fs::remove_file(&p);
                }
            }
            Some("tmp") => {
                let _ = fs::remove_file(&p);
            }
            _ => {}
        }
    }
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.p.checked_add(n)?;
        let s = self.b.get(self.p..end)?;
        self.p = end;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    }
}

/// Body of a file if its trailing checksum matches.
fn verify(b: &[u8]) -> Option<&[u8]> {
    if b.len() < 8 {
        return None;
    }
    let (body, sum) = b.split_at(b.len() - 8);
    (xxh3_64(body) == u64::from_le_bytes(sum.try_into().unwrap())).then_some(body)
}

pub fn parse_item(b: &[u8]) -> Option<Payload> {
    let mut r = Rd { b: verify(b)?, p: 0 };
    if r.take(4)? != ITEM_MAGIC || r.u32()? != VERSION {
        return None;
    }
    let n = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let id = r.u32()?;
        let nl = r.u16()? as usize;
        let name = String::from_utf8_lossy(r.take(nl)?).into_owned();
        let dl = usize::try_from(r.u64()?).ok()?;
        let data = r.take(dl)?.to_vec();
        out.push(Fmt { id, name, data });
    }
    Some(out)
}

pub fn parse_index(b: &[u8]) -> Option<Vec<Entry>> {
    let mut r = Rd { b: verify(b)?, p: 0 };
    if r.take(4)? != INDEX_MAGIC || r.u32()? != VERSION {
        return None;
    }
    let n = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let id = r.u64()?;
        let time = r.u64()?;
        let sl = r.u16()? as usize;
        let source = String::from_utf8_lossy(r.take(sl)?).into_owned();
        out.push(Entry { id, time, source });
    }
    Some(out)
}

pub fn read_item(dir: &Path, id: u64) -> Option<Payload> {
    parse_item(&fs::read(item_path(dir, id)).ok()?)
}

/// Load history (newest first). If the index is missing or broken, fall back
/// to item files ordered by modification time.
pub fn load(dir: &Path, max: usize) -> Vec<(Entry, Payload)> {
    let entries = fs::read(dir.join("index.dat")).ok().and_then(|b| parse_index(&b)).unwrap_or_else(|| scan(dir));
    entries
        .into_iter()
        .filter_map(|e| read_item(dir, e.id).map(|p| (e, p)))
        .take(max)
        .collect()
}

fn scan(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<Entry> = rd
        .flatten()
        .filter_map(|f| {
            let p = f.path();
            if p.extension()? != "item" {
                return None;
            }
            let id = u64::from_str_radix(p.file_stem()?.to_str()?, 16).ok()?;
            let time = f.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some(Entry { id, time, source: String::new() })
        })
        .collect();
    v.sort_by(|a, b| b.time.cmp(&a.time));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("unshufflz-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Payload {
        vec![
            Fmt { id: 13, name: String::new(), data: vec![b'h', 0, b'i', 0, 0, 0] },
            Fmt { id: 0xC0AB, name: "HTML Format".into(), data: b"<b>hi</b>".to_vec() },
        ]
    }

    #[test]
    fn item_roundtrip() {
        let d = tmp_dir("item");
        write_item(&d, 42, &sample()).unwrap();
        let p = read_item(&d, 42).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].id, 13);
        assert_eq!(p[0].data, sample()[0].data);
        assert_eq!(p[1].name, "HTML Format");
        assert_eq!(p[1].data, b"<b>hi</b>");
        assert!(!d.join(format!("{:016x}.tmp", 42)).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_item_rejected() {
        let d = tmp_dir("corrupt");
        write_item(&d, 1, &sample()).unwrap();
        let path = item_path(&d, 1);
        let mut b = fs::read(&path).unwrap();
        b[14] ^= 0xFF;
        assert!(parse_item(&b).is_none());
        b[14] ^= 0xFF;
        assert!(parse_item(&b).is_some());
        // truncated file (as after a crash mid-write)
        assert!(parse_item(&b[..b.len() / 2]).is_none());
        assert!(parse_item(&[]).is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn index_roundtrip_and_gc() {
        let d = tmp_dir("index");
        write_item(&d, 1, &sample()).unwrap();
        write_item(&d, 2, &sample()).unwrap();
        fs::write(d.join("dead.tmp"), b"x").unwrap();
        let e = vec![Entry { id: 2, time: 100, source: "notepad.exe".into() }];
        write_index(&d, &e).unwrap();
        gc(&d, &e);
        assert!(!item_path(&d, 1).exists());
        assert!(item_path(&d, 2).exists());
        assert!(!d.join("dead.tmp").exists());
        let loaded = load(&d, 10);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, e[0]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn broken_index_falls_back_to_scan() {
        let d = tmp_dir("scan");
        write_item(&d, 7, &sample()).unwrap();
        fs::write(d.join("index.dat"), b"garbage-garbage").unwrap();
        let loaded = load(&d, 10);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0.id, 7);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn writer_thread_persists() {
        let d = tmp_dir("writer");
        let mut s = Store::start(d.clone(), 0, 0);
        s.save_item(9, Arc::new(sample()));
        s.save_index(vec![Entry { id: 9, time: 1, source: String::new() }]);
        s.shutdown();
        s.shutdown();
        assert_eq!(load(&d, 10).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }
}
