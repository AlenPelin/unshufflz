/// Settings from `unshufflz.ini` next to the exe (all optional):
///   HoldDelayMs=250   WidthPercent=15   MaxItemMB=200
pub struct Config {
    pub hold_ms: u32,
    pub width_pct: u32,
    pub max_item_bytes: u64,
}

pub fn load() -> Config {
    let path = crate::util::exe_path().with_file_name("unshufflz.ini");
    parse(&std::fs::read_to_string(path).unwrap_or_default())
}

pub fn parse(text: &str) -> Config {
    let mut c = Config { hold_ms: 250, width_pct: 15, max_item_bytes: 200 << 20 };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let Ok(v) = v.trim().parse::<u64>() else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "holddelayms" => c.hold_ms = v.clamp(50, 2000) as u32,
            "widthpercent" => c.width_pct = v.clamp(5, 50) as u32,
            "maxitemmb" => c.max_item_bytes = v.clamp(1, 2048) << 20,
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let c = parse("");
        assert_eq!((c.hold_ms, c.width_pct, c.max_item_bytes), (250, 15, 200 << 20));
        let c = parse("[General]\nHoldDelayMs = 400\nwidthpercent=20\nMaxItemMB=5\njunk\nOther=1");
        assert_eq!((c.hold_ms, c.width_pct, c.max_item_bytes), (400, 20, 5 << 20));
    }

    #[test]
    fn values_are_clamped() {
        let c = parse("HoldDelayMs=1\nWidthPercent=99\nMaxItemMB=0");
        assert_eq!((c.hold_ms, c.width_pct, c.max_item_bytes), (50, 50, 1 << 20));
    }
}
