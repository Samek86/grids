use super::*;

// Resolved UI language: 0 English, 1 Korean, 2 Japanese. The preference (including Auto) lives on the session.
pub(crate) static UI: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lang {
    Auto,
    En,
    Ko,
    Ja,
}

impl Lang {
    pub(crate) fn from_ini(v: &str) -> Self {
        match v.trim() {
            "en" => Lang::En,
            "ko" => Lang::Ko, // also the old on/off value
            "ja" => Lang::Ja,
            _ => Lang::Auto,
        }
    }

    pub(crate) fn ini(self) -> &'static str {
        match self {
            Lang::Auto => "auto",
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Ja => "ja",
        }
    }

    /// Auto follows the Windows UI language: Korean, Japanese, otherwise English.
    pub(crate) fn resolve(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::Ko => 1,
            Lang::Ja => 2,
            Lang::Auto => {
                let prim = unsafe { GetUserDefaultUILanguage() } & 0x3FF;
                if prim == 0x12 {
                    1
                } else if prim == 0x11 {
                    2
                } else {
                    0
                }
            }
        }
    }
}

pub(crate) fn ui() -> u8 {
    UI.load(Ordering::Relaxed)
}

/// UI text. Explorer's own strings (column names, shell menus) stay with Windows.
pub(crate) fn tr(en: &'static str, ko: &'static str, ja: &'static str) -> &'static str {
    match ui() {
        1 => ko,
        2 => ja,
        _ => en,
    }
}

/// Face used when no `font_face` is chosen. Japanese is Yu Gothic UI.
pub(crate) fn default_face(lang: u8) -> &'static str {
    match lang {
        1 => "Malgun Gothic",
        2 => "Yu Gothic UI",
        _ => "Segoe UI",
    }
}

pub(crate) fn ui_face() -> &'static str {
    default_face(ui())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_face_follows_the_language() {
        assert_eq!(default_face(0), "Segoe UI");
        assert_eq!(default_face(1), "Malgun Gothic");
        assert_eq!(default_face(2), "Yu Gothic UI");
        assert_eq!(default_face(9), "Segoe UI");
        let prev = UI.load(Ordering::Relaxed);
        UI.store(2, Ordering::Relaxed);
        assert_eq!(ui_face(), "Yu Gothic UI");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(ui_face(), "Malgun Gothic");
        UI.store(0, Ordering::Relaxed);
        assert_eq!(ui_face(), "Segoe UI");
        UI.store(prev, Ordering::Relaxed);
    }
}

pub(crate) const ACCENT_EN: [&str; 6] = ["Sky", "Gray", "Lavender", "Sage", "Peach", "Rose"];
pub(crate) const ACCENT_JA: [&str; 6] = ["空", "グレー", "ラベンダー", "セージ", "ピーチ", "ローズ"];
