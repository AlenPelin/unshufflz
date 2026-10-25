/// Settings in `%LOCALAPPDATA%\Unshufflz\unshufflz.ini` (all keys optional):
///   HoldDelayMs=0   WidthPercent=15   Position=right   MaxItems=10   MaxItemMB=200   PicturesDir=
///   ShowWithStart=0   Animation=1   AnimationMs=50
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
    /// Also show the overlay while the Start menu is open.
    pub show_with_start: bool,
    /// Slide-in animation on/off.
    pub anim_on: bool,
    /// Slide-in duration (kept when the animation is turned off); 0 = no animation.
    pub anim_ms: u32,
}

impl Config {
    /// Effective slide-in duration: 0 when the animation is off.
    pub fn slide_ms(&self) -> u32 {
        if self.anim_on { self.anim_ms } else { 0 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { hold_ms: 0, width_pct: 15, side: Side::Right, max_items: 10, max_item_bytes: 200 << 20, pictures_dir: String::new(), show_with_start: false, anim_on: true, anim_ms: 50 }
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
            ("showwithstart", Some(n)) => c.show_with_start = n != 0,
            ("animation", Some(n)) => c.anim_on = n != 0,
            ("animationms", Some(n)) => c.anim_ms = n.min(1000) as u32,
            ("position", _) if v.eq_ignore_ascii_case("left") => c.side = Side::Left,
            ("position", _) if v.eq_ignore_ascii_case("right") => c.side = Side::Right,
            _ => {}
        }
    }
    c
}

pub fn format(c: &Config) -> String {
    format!(
        "[General]\r\nHoldDelayMs={}\r\nWidthPercent={}\r\nPosition={}\r\nMaxItems={}\r\nMaxItemMB={}\r\nPicturesDir={}\r\nShowWithStart={}\r\nAnimation={}\r\nAnimationMs={}\r\n",
        c.hold_ms,
        c.width_pct,
        if c.side == Side::Left { "left" } else { "right" },
        c.max_items,
        c.max_item_bytes >> 20,
        c.pictures_dir,
        c.show_with_start as u8,
        c.anim_on as u8,
        c.anim_ms
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
        let c = Config { hold_ms: 300, width_pct: 22, side: Side::Left, max_items: 42, max_item_bytes: 7 << 20, pictures_dir: r"D:\My Shots=1\x".into(), show_with_start: true, anim_on: false, anim_ms: 120 };
        assert_eq!(parse(&format(&c)), c);
        assert_eq!(parse("PicturesDir=").pictures_dir, "");
        assert!(parse("ShowWithStart=1").show_with_start);
        assert!(!parse("ShowWithStart=0").show_with_start);
        assert!(!parse("").show_with_start);
        assert_eq!(parse("").anim_ms, 50);
        assert_eq!(parse("AnimationMs=0").anim_ms, 0);
        assert_eq!(parse("AnimationMs=99999").anim_ms, 1000);
        // on/off keeps the duration
        let c = parse("Animation=0\nAnimationMs=200");
        assert_eq!((c.anim_on, c.anim_ms, c.slide_ms()), (false, 200, 0));
        let c = parse("Animation=1\nAnimationMs=200");
        assert_eq!(c.slide_ms(), 200);
        assert!(parse("").anim_on);
        assert_eq!(parse("picturesdir =  C:\\a b  ").pictures_dir, "C:\\a b");
    }
}
