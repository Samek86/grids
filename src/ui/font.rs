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

// Windows' own UI font at this DPI (no custom fonts: they only reached our chrome and cost paint time),
// plus the MDL2 icon font for the buttons. Pushed to every edit; also used on DPI change.
pub(crate) unsafe fn apply_font_all() {
    across_windows(|| apply_font());
}

pub(crate) unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont("Segoe UI", 12 * dpi / 96) };
    lf.lfFaceName = logfont(ui_face(), 0).lfFaceName; // Segoe UI / Malgun Gothic / Yu Gothic UI
    let scale = match app(|a| a.s.font) {
        1 => 125,
        2 => 150,
        _ => 100,
    };
    lf.lfHeight = lf.lfHeight * scale / 100;
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let icon_font = CreateFontIndirectW(&logfont("Segoe MDL2 Assets", 14 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, old_bold, panes, edit) = app(|a| {
        a.fh = tm.tmHeight;
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { _ = DeleteObject(old_small.into()) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        // Group labels clone this face on the next paint. Drop the previous one now so a DPI
        // or font-scale change cannot keep a bold font at the old size.
        let old_bold = std::mem::replace(&mut a.bold_font, HFONT::default());
        let mut panes: Vec<HWND> = a.panes.iter().flatten().map(|p| p.hwnd).collect();
        for set in &a.park {
            panes.extend(set.iter().flatten().map(|p| p.hwnd));
        }
        (old, old_icon, old_bold, panes, a.tab_edit)
    });
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
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
    relayout();
    redraw_all();
}

/// Bold copy of the window's UI font. Built once, the first time a group row is painted,
/// and thrown away by `apply_font` (DPI, font scale) or when the window closes.
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

