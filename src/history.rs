use crate::clip::{Payload, CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use crate::preview::Preview;
use std::sync::Arc;
use xxhash_rust::xxh3::Xxh3;

pub const MAX: usize = 10;
/// Items bigger than this keep their payload only on disk once saved.
pub const RAM_LIMIT: u64 = 1 << 20;

pub struct Item {
    /// Hash of all formats; also the file name in the store.
    pub id: u64,
    /// Hash of the main content (files / image / text); used to find duplicates.
    pub key: u64,
    pub time: u64,
    pub source: String,
    pub size: u64,
    pub payload: Option<Arc<Payload>>,
    pub preview: Preview,
}

fn hash_fmt(h: &mut Xxh3, f: &crate::clip::Fmt) {
    if f.name.is_empty() {
        h.update(&f.id.to_le_bytes());
    } else {
        h.update(f.name.as_bytes());
    }
    h.update(&(f.data.len() as u64).to_le_bytes());
    h.update(&f.data);
}

pub fn full_hash(p: &Payload) -> u64 {
    let mut h = Xxh3::new();
    for f in p {
        hash_fmt(&mut h, f);
    }
    h.digest()
}

pub fn content_key(p: &Payload) -> u64 {
    let main = [CF_HDROP, CF_DIBV5, CF_DIB, CF_UNICODETEXT]
        .iter()
        .find_map(|&id| p.iter().find(|f| f.id == id));
    match main {
        Some(f) => {
            let mut h = Xxh3::new();
            hash_fmt(&mut h, f);
            h.digest()
        }
        None => full_hash(p),
    }
}

pub fn payload_size(p: &Payload) -> u64 {
    p.iter().map(|f| f.data.len() as u64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Fmt;

    fn f(id: u32, name: &str, data: &[u8]) -> Fmt {
        Fmt { id, name: name.into(), data: data.to_vec() }
    }

    #[test]
    fn same_text_with_extra_formats_is_duplicate() {
        let plain = vec![f(CF_UNICODETEXT, "", b"h\0i\0\0\0")];
        let rich = vec![f(CF_UNICODETEXT, "", b"h\0i\0\0\0"), f(0xC100, "HTML Format", b"<b>hi</b>")];
        assert_eq!(content_key(&plain), content_key(&rich));
        assert_ne!(full_hash(&plain), full_hash(&rich));
    }

    #[test]
    fn registered_formats_hash_by_name_not_id() {
        let a = vec![f(0xC100, "X", b"1")];
        let b = vec![f(0xC222, "X", b"1")];
        assert_eq!(full_hash(&a), full_hash(&b));
        assert_eq!(payload_size(&a), 1);
    }

    #[test]
    fn different_text_differs() {
        let a = vec![f(CF_UNICODETEXT, "", b"a\0\0\0")];
        let b = vec![f(CF_UNICODETEXT, "", b"b\0\0\0")];
        assert_ne!(content_key(&a), content_key(&b));
    }
}
