use crate::imports::*;

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
pub(crate) struct Palette {
    pub(crate) bg: u32,   // frame: window, toolbar, gaps, status line
    pub(crate) card: u32, // plates and lists
    pub(crate) line: u32,
    pub(crate) pill: u32, // fields
    pub(crate) text: u32,
    pub(crate) sub: u32,
    pub(crate) accents: [(&'static str, u32); 6],
}

pub(crate) const LIGHT: Palette = Palette {
    bg: 0xF3F1EC,
    card: 0xFFFFFF,
    line: 0xE4E0D8,
    pill: 0xF4F2EE,
    text: 0x2A2825,
    sub: 0x7C776F,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

pub(crate) const DARK: Palette = Palette {
    bg: 0x1C1D20,
    card: 0x26282C,
    line: 0x36393E,
    pill: 0x303338,
    text: 0xE6E8EB,
    sub: 0x9AA0A8,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

pub(crate) static DARK_ON: AtomicBool = AtomicBool::new(false);

pub(crate) fn pal() -> &'static Palette {
    if DARK_ON.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

pub(crate) const INK: u32 = 0x2A2825; // dark text on light selection colours
pub(crate) const GREEN: u32 = 0x1FA463; // unsaved layout changes
pub(crate) fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

/// What the left panel (folder tree and favourites) sits on: the app background in light, the list colour in dark.
pub(crate) fn side_ground() -> u32 {
    if DARK_ON.load(std::sync::atomic::Ordering::Relaxed) {
        pal().card
    } else {
        pal().bg
    }
}

/// Favourite group row wash: accent mixed into the panel. Light keeps about 18% accent.
/// Dark uses a smaller share so a bright accent stays a dim tint rather than a glow.
pub(crate) fn group_tint(accent: u32, ground: u32, dark: bool) -> u32 {
    blend(ground, accent, if dark { 0.12 } else { 0.18 })
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
pub(crate) fn on_color(bg: u32) -> u32 {
    let l = 0.2126 * (bg >> 16 & 0xFF) as f32 + 0.7152 * (bg >> 8 & 0xFF) as f32 + 0.0722 * (bg & 0xFF) as f32;
    if l > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: u32, b: u32) -> u32 {
        let d = |s: u32| (a >> s & 0xFF).abs_diff(b >> s & 0xFF) as u32;
        d(16) + d(8) + d(0)
    }

    fn lum(c: u32) -> f32 {
        0.2126 * (c >> 16 & 0xFF) as f32 + 0.7152 * (c >> 8 & 0xFF) as f32 + 0.0722 * (c & 0xFF) as f32
    }

    #[test]
    fn group_tint_is_a_light_wash_and_selection_is_stronger() {
        let accent = 0x0067C0;
        let tint = group_tint(accent, LIGHT.card, false);
        let sel = blend(accent, LIGHT.card, 0.16);
        let hot = blend(LIGHT.bg, LIGHT.card, 0.35);
        assert_eq!(tint, blend(LIGHT.card, accent, 0.18));
        assert_ne!(tint, hot);
        assert!(dist(sel, LIGHT.card) > dist(tint, LIGHT.card) * 2);

        let accent_d = 0x4CC2FF;
        let tint_d = group_tint(accent_d, DARK.card, true);
        let sel_d = blend(accent_d, DARK.card, 0.16);
        let hot_d = blend(DARK.bg, DARK.card, 0.35);
        assert_eq!(tint_d, blend(DARK.card, accent_d, 0.12));
        assert!(lum(tint_d) < lum(tint));
        assert_ne!(tint_d, hot_d);
        assert!(dist(sel_d, DARK.card) > dist(tint_d, DARK.card) * 2);
    }
}
