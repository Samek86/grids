use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::i18n::*;

// ---------- fonts ----------

pub(crate) fn logfont(face: &str, px: i32) -> LOGFONTW {
    let mut lf = LOGFONTW { lfHeight: -px, lfCharSet: DEFAULT_CHARSET, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    lf
}

fn utf16_face(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len()).min(31);
    String::from_utf16_lossy(&buf[..n])
}

/// LOGFONT stores 31 UTF-16 units plus a NUL. Longer names cannot be selected.
fn fits_face(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.starts_with('@') && name.encode_utf16().nth(31).is_none()
}

pub(crate) fn same_face(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a.eq_ignore_ascii_case(b) || a.chars().flat_map(|c| c.to_lowercase()).eq(b.chars().flat_map(|c| c.to_lowercase()))
}

/// `stored` when it is in `installed`, otherwise the language default. Empty stored is the default.
/// `installed` is the caller's list. This does not ask Windows.
pub(crate) fn resolve_face<'a>(stored: &str, lang: u8, installed: &'a [&'a str]) -> &'a str {
    let stored = stored.trim();
    if fits_face(stored) {
        if let Some(hit) = installed.iter().copied().find(|n| same_face(n, stored)) {
            return hit;
        }
    }
    default_face(lang)
}

struct FaceHit {
    name: String,
    proportional: bool,
    raster: bool,
}

fn face_rank(h: &FaceHit) -> u8 {
    // Prefer a proportional outline over a fixed-pitch or raster duplicate of the same name.
    u8::from(h.proportional) << 1 | u8::from(!h.raster)
}

fn face_key(name: &str) -> Vec<char> {
    name.trim().chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Families for the Settings list: no '@' names, one row per name, proportional outline wins a duplicate.
fn family_names(hits: &[FaceHit]) -> Vec<String> {
    let mut best: Vec<FaceHit> = Vec::new();
    for h in hits {
        if !fits_face(&h.name) {
            continue;
        }
        let key = face_key(&h.name);
        if let Some(i) = best.iter().position(|e| face_key(&e.name) == key) {
            if face_rank(h) > face_rank(&best[i]) {
                best[i] = FaceHit { name: h.name.trim().to_string(), proportional: h.proportional, raster: h.raster };
            }
            continue;
        }
        best.push(FaceHit { name: h.name.trim().to_string(), proportional: h.proportional, raster: h.raster });
    }
    best.sort_by(|a, b| face_key(&a.name).cmp(&face_key(&b.name)));
    best.into_iter().map(|h| h.name).collect()
}

struct Probe<'a> {
    want: &'a str,
    found: bool,
}

unsafe extern "system" fn probe_face(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _ty: u32, lp: LPARAM) -> i32 {
    if lf.is_null() || lp.0 == 0 {
        return 1;
    }
    let probe = &mut *(lp.0 as *mut Probe);
    let got = utf16_face(&(*lf).lfFaceName);
    if same_face(&got, probe.want) {
        probe.found = true;
        return 0;
    }
    1
}

/// One family lookup. Does not build the Settings list.
pub(crate) unsafe fn face_installed(name: &str) -> bool {
    let name = name.trim();
    if !fits_face(name) {
        return false;
    }
    let mut lf = logfont(name, 0);
    lf.lfCharSet = DEFAULT_CHARSET;
    let mut probe = Probe { want: name, found: false };
    let dc = GetDC(None);
    if dc.is_invalid() {
        return false;
    }
    let _ = EnumFontFamiliesExW(dc, &lf, Some(probe_face), LPARAM(&mut probe as *mut Probe as isize), 0);
    ReleaseDC(None, dc);
    probe.found
}

unsafe extern "system" fn collect_face(lf: *const LOGFONTW, tm: *const TEXTMETRICW, ty: u32, lp: LPARAM) -> i32 {
    if lf.is_null() || lp.0 == 0 {
        return 1;
    }
    let name = utf16_face(&(*lf).lfFaceName);
    if name.is_empty() {
        return 1;
    }
    let proportional = !tm.is_null() && (*tm).tmPitchAndFamily.0 & TMPF_FIXED_PITCH.0 != 0;
    let hits = &mut *(lp.0 as *mut Vec<FaceHit>);
    hits.push(FaceHit { name, proportional, raster: ty & RASTER_FONTTYPE != 0 });
    1
}

/// Installed families. Call only while Settings is open; the caller drops the `Vec` when it closes.
pub(crate) unsafe fn installed_families() -> Vec<String> {
    let lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
    let mut hits = Vec::new();
    let dc = GetDC(None);
    if !dc.is_invalid() {
        let _ = EnumFontFamiliesExW(dc, &lf, Some(collect_face), LPARAM(&mut hits as *mut Vec<FaceHit> as isize), 0);
        ReleaseDC(None, dc);
    }
    family_names(&hits)
}

// Windows' UI font metrics at this DPI, with the language default or the face chosen in Settings.
// The MDL2 icon font stays on the buttons. Pushed to every edit; also used on DPI change.
pub(crate) unsafe fn apply_font_all() {
    across_windows(|| apply_font());
}

pub(crate) unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    // Empty stays on the language default and does not look up a face. A missing name is cleared.
    let stored = app(|a| if a.s.font_face.is_empty() { None } else { Some(a.s.font_face.clone()) });
    let custom = match stored.as_deref() {
        Some(name) if face_installed(name) => true,
        Some(_) => {
            app(|a| a.s.font_face.clear());
            false
        }
        None => false,
    };
    let slot = stored.as_deref().unwrap_or("");
    let list = [slot];
    let none: &[&str] = &[];
    let face = if custom { resolve_face(slot, ui(), &list) } else { resolve_face("", ui(), none) };
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont(ui_face(), 12 * dpi / 96) };
    lf.lfFaceName = logfont(face, 0).lfFaceName;
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let icon_font = CreateFontIndirectW(&logfont("Segoe MDL2 Assets", 14 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, old_bold, panes, edit, load_edit) = app(|a| {
        a.fh = tm.tmHeight;
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { _ = DeleteObject(old_small.into()) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        // Group labels clone this face on the next paint. Drop the previous one now so a DPI
        // change cannot keep a bold font at the old size.
        let old_bold = std::mem::replace(&mut a.bold_font, HFONT::default());
        let mut panes: Vec<HWND> = a.panes.iter().flatten().map(|p| p.hwnd).collect();
        for set in &a.park {
            panes.extend(set.iter().flatten().map(|p| p.hwnd));
        }
        (old, old_icon, old_bold, panes, a.tab_edit, a.load_edit)
    });
    for e in [edit, load_edit] {
        if !e.0.is_null() {
            SendMessageW(e, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
    }
    for p in panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    for f in [old, old_icon, old_bold].into_iter().filter(|f| !f.is_invalid()) {
        _ = DeleteObject(f.into());
    }
    // The folder tree takes this font, so its item height (and the favourites rows) follow DPI.
    // Preview headings were cloned from the previous face; the next paint builds them again.
    drop_peek_fonts();
    super::sync_fav_metrics();
    relayout();
    redraw_all();
}

/// Bold copy of the window's UI font. Built once, the first time a group row is painted,
/// and thrown away by `apply_font` (a DPI or face change) or when the window closes.
pub(crate) unsafe fn bold_ui_font() -> HFONT {
    let cached = app(|a| a.bold_font);
    if !cached.is_invalid() {
        return cached;
    }
    let base = app(|a| a.font);
    if base.is_invalid() {
        return base;
    }
    let mut lf = LOGFONTW::default();
    let n = GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut LOGFONTW as *mut _));
    if n < size_of::<LOGFONTW>() as i32 {
        return base;
    }
    lf.lfWeight = FW_BOLD.0 as i32;
    let font = CreateFontIndirectW(&lf);
    if font.is_invalid() {
        return base;
    }
    let stored = app(|a| {
        if a.bold_font.is_invalid() {
            a.bold_font = font;
            font
        } else {
            a.bold_font
        }
    });
    if stored != font {
        _ = DeleteObject(font.into());
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_face_falls_back_to_the_language_default() {
        let installed = ["Segoe UI", "Malgun Gothic", "Yu Gothic UI", "Consolas"];
        assert_eq!(resolve_face("", 0, &installed), "Segoe UI");
        assert_eq!(resolve_face("   ", 1, &installed), "Malgun Gothic");
        assert_eq!(resolve_face("Nope", 2, &installed), "Yu Gothic UI");
        assert_eq!(resolve_face("consolas", 0, &installed), "Consolas");
        assert_eq!(resolve_face("Yu Gothic UI", 1, &installed), "Yu Gothic UI");
        assert_eq!(resolve_face("@Yu Gothic UI", 2, &installed), "Yu Gothic UI");
        assert_eq!(crate::default_face(2), "Yu Gothic UI");
    }

    #[test]
    fn family_list_skips_vertical_and_nonproportional_duplicates() {
        let hits = [
            FaceHit { name: "@Arial".into(), proportional: true, raster: false },
            FaceHit { name: "Arial".into(), proportional: false, raster: true },
            FaceHit { name: "Arial".into(), proportional: true, raster: false },
            FaceHit { name: "consolas".into(), proportional: false, raster: false },
            FaceHit { name: "Consolas".into(), proportional: false, raster: false },
            FaceHit { name: "Yu Gothic UI".into(), proportional: true, raster: false },
            FaceHit { name: String::new(), proportional: true, raster: false },
        ];
        assert_eq!(family_names(&hits), ["Arial".to_string(), "consolas".into(), "Yu Gothic UI".into()]);
    }

    #[test]
    fn installed_families_are_unique_sorted_and_not_vertical() {
        let names = unsafe { installed_families() };
        let mut sorted = names.clone();
        sorted.sort_by(|a, b| face_key(a).cmp(&face_key(b)));
        assert_eq!(names, sorted);
        let mut seen = Vec::new();
        for n in &names {
            assert!(fits_face(n), "{n}");
            let key = face_key(n);
            assert!(!seen.contains(&key), "duplicate {n}");
            seen.push(key);
        }
    }
}

