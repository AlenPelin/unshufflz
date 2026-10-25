/// Settings in `%LOCALAPPDATA%\Unshufflz\unshufflz.ini` (all keys optional):
///   HoldDelayMs=0   WidthPercent=15   Position=right   MaxItems=10   MaxItemMB=200   PicturesDir=
/// The settings window rewrites this file.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hold_ms: u32,
    pub width_pct: u32,
    pub side: Side,
    pub max_items: usize,
    pub max_item_bytes: u64,
    /// Folder for Win+S pictures; empty = Pictures\Clipboard.
    pub pictures_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { hold_ms: 0, width_pct: 15, side: Side::Right, max_items: 10, max_item_bytes: 200 << 20, pictures_dir: String::new() }
    }
}

pub const MAX_ITEMS_LIMIT: usize = 100;

pub fn path() -> std::path::PathBuf {
    crate::util::data_dir().join("unshufflz.ini")
}

pub fn load() -> Config {
    parse(&std::fs::read_to_string(path()).unwrap_or_default())
}

pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let n = v.parse::<u64>().ok();
        match (k.trim().to_ascii_lowercase().as_str(), n) {
            ("holddelayms", Some(n)) => c.hold_ms = n.min(2000) as u32,
            ("widthpercent", Some(n)) => c.width_pct = n.clamp(5, 50) as u32,
            ("maxitems", Some(n)) => c.max_items = n.clamp(1, MAX_ITEMS_LIMIT as u64) as usize,
            ("maxitemmb", Some(n)) => c.max_item_bytes = n.clamp(1, 2048) << 20,
            ("picturesdir", _) => c.pictures_dir = v.to_string(),
            ("position", _) if v.eq_ignore_ascii_case("left") => c.side = Side::Left,
            ("position", _) if v.eq_ignore_ascii_case("right") => c.side = Side::Right,
            _ => {}
        }
    }
    c
}

pub fn format(c: &Config) -> String {
    format!(
        "[General]\r\nHoldDelayMs={}\r\nWidthPercent={}\r\nPosition={}\r\nMaxItems={}\r\nMaxItemMB={}\r\nPicturesDir={}\r\n",
        c.hold_ms,
        c.width_pct,
        if c.side == Side::Left { "left" } else { "right" },
        c.max_items,
        c.max_item_bytes >> 20,
        c.pictures_dir
    )
}

pub fn save(c: &Config) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::util::write_file_safe(&p, format(c).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(parse(""), Config::default());
        let c = parse("[General]\nHoldDelayMs = 400\nwidthpercent=20\nPosition = Left\nMaxItems=25\nMaxItemMB=5\njunk\nOther=1");
        assert_eq!((c.hold_ms, c.width_pct, c.side, c.max_items, c.max_item_bytes), (400, 20, Side::Left, 25, 5 << 20));
    }

    #[test]
    fn values_are_clamped() {
        let c = parse("HoldDelayMs=5000\nWidthPercent=99\nMaxItemMB=0\nPosition=middle\nMaxItems=1000");
        assert_eq!((c.hold_ms, c.width_pct, c.side, c.max_items, c.max_item_bytes), (2000, 50, Side::Right, 100, 1 << 20));
        assert_eq!(parse("HoldDelayMs=0").hold_ms, 0);
        assert_eq!(parse("MaxItems=0").max_items, 1);
    }

    #[test]
    fn format_roundtrip() {
        let c = Config { hold_ms: 300, width_pct: 22, side: Side::Left, max_items: 42, max_item_bytes: 7 << 20, pictures_dir: r"D:\My Shots=1\x".into() };
        assert_eq!(parse(&format(&c)), c);
        assert_eq!(parse("PicturesDir=").pictures_dir, "");
        assert_eq!(parse("picturesdir =  C:\\a b  ").pictures_dir, "C:\\a b");
    }
}
